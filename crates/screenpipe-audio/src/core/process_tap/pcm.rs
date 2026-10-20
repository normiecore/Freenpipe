// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Sample-format handling for loopback buffers: describe a WAVEFORMATEX(-
//! TENSIBLE) layout, turn a raw packet into mono `f32`, and keep a stream's
//! output rate fixed when a rebuilt client runs at a different mix rate.
//! Pure code, so it is tested on every platform.

/// `WAVE_FORMAT_PCM`.
pub const FORMAT_TAG_PCM: u16 = 0x0001;
/// `WAVE_FORMAT_IEEE_FLOAT`.
pub const FORMAT_TAG_IEEE_FLOAT: u16 = 0x0003;
/// `WAVE_FORMAT_EXTENSIBLE`: the real format is in the sub-format GUID,
/// whose first field repeats one of the tags above.
pub const FORMAT_TAG_EXTENSIBLE: u16 = 0xFFFE;

/// How one sample is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleEncoding {
    F32,
    F64,
    U8,
    I16,
    /// 24-bit samples packed in 3 bytes.
    I24Packed,
    /// 32-bit containers. `valid_bits` 24 means 24-bit audio left-aligned in
    /// the container, which reads the same as full 32-bit audio.
    I32,
}

/// Everything needed to decode a packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleLayout {
    pub encoding: SampleEncoding,
    pub channels: u16,
    pub sample_rate: u32,
    /// Bytes per frame (all channels).
    pub block_align: u16,
}

/// The fields of a WAVEFORMATEX the decoder needs, plus the first field of
/// the sub-format GUID when the tag is `WAVE_FORMAT_EXTENSIBLE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatDescription {
    pub format_tag: u16,
    pub channels: u16,
    pub sample_rate: u32,
    pub block_align: u16,
    pub bits_per_sample: u16,
    pub subformat_tag: Option<u32>,
}

impl SampleLayout {
    /// Interprets a format description. Errors name what is unsupported.
    pub fn from_format(desc: &FormatDescription) -> Result<Self, String> {
        if desc.channels == 0 {
            return Err("audio format has no channels".to_string());
        }
        if desc.sample_rate == 0 {
            return Err("audio format has a zero sample rate".to_string());
        }
        let tag = if desc.format_tag == FORMAT_TAG_EXTENSIBLE {
            match desc.subformat_tag {
                Some(sub) if sub <= u16::MAX as u32 => sub as u16,
                _ => return Err("extensible audio format without a known sub-format".to_string()),
            }
        } else {
            desc.format_tag
        };
        let container_bytes = desc.block_align as usize / desc.channels as usize;
        let encoding = match (tag, container_bytes) {
            (FORMAT_TAG_IEEE_FLOAT, 4) => SampleEncoding::F32,
            (FORMAT_TAG_IEEE_FLOAT, 8) => SampleEncoding::F64,
            (FORMAT_TAG_PCM, 1) => SampleEncoding::U8,
            (FORMAT_TAG_PCM, 2) => SampleEncoding::I16,
            (FORMAT_TAG_PCM, 3) => SampleEncoding::I24Packed,
            (FORMAT_TAG_PCM, 4) => SampleEncoding::I32,
            _ => {
                return Err(format!(
                "unsupported audio format: tag {:#06x}, {} bits, block align {} for {} channels",
                tag, desc.bits_per_sample, desc.block_align, desc.channels
            ))
            }
        };
        if desc.block_align as usize != container_bytes * desc.channels as usize {
            return Err(format!(
                "block align {} is not a whole number of {}-channel frames",
                desc.block_align, desc.channels
            ));
        }
        Ok(Self {
            encoding,
            channels: desc.channels,
            sample_rate: desc.sample_rate,
            block_align: desc.block_align,
        })
    }

    fn bytes_per_sample(&self) -> usize {
        self.block_align as usize / self.channels as usize
    }

    /// Decodes `frames` frames from `data` into one mono sample per frame
    /// (the channel average). Trailing bytes beyond `frames` are ignored and
    /// a short buffer decodes as many whole frames as it holds.
    pub fn decode_mono(&self, data: &[u8], frames: usize) -> Vec<f32> {
        let frame_bytes = self.block_align as usize;
        let usable = frames.min(data.len() / frame_bytes.max(1));
        let width = self.bytes_per_sample();
        let channels = self.channels as usize;
        let mut out = Vec::with_capacity(usable);
        for frame in data.chunks_exact(frame_bytes).take(usable) {
            let mut sum = 0.0f32;
            for ch in 0..channels {
                let s = &frame[ch * width..(ch + 1) * width];
                sum += self.decode_sample(s);
            }
            out.push(sum / channels as f32);
        }
        out
    }

    fn decode_sample(&self, s: &[u8]) -> f32 {
        match self.encoding {
            SampleEncoding::F32 => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
            SampleEncoding::F64 => {
                f64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]) as f32
            }
            SampleEncoding::U8 => (s[0] as f32 - 128.0) / 128.0,
            SampleEncoding::I16 => i16::from_le_bytes([s[0], s[1]]) as f32 / 32_768.0,
            SampleEncoding::I24Packed => {
                // Sign-extend by placing the 3 bytes in the top of an i32.
                let v = i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8;
                v as f32 / 8_388_608.0
            }
            SampleEncoding::I32 => {
                (i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f64 / 2_147_483_648.0) as f32
            }
        }
    }
}

/// Streaming linear-interpolation resampler for mono audio.
///
/// A stream reports one sample rate for its whole life, but after a rebuild
/// (new default device, exclusion switch) the client may run at another mix
/// rate. This converts such a client's output back to the stream rate. It
/// keeps its position across calls, so chunk boundaries do not click.
/// Speech recognition resamples to 16 kHz afterwards, so linear
/// interpolation is plenty.
#[derive(Debug, Clone)]
pub struct LinearResampler {
    from_rate: u32,
    to_rate: u32,
    /// Read position, in input samples, relative to the start of the next
    /// input chunk (negative while it still sits on `last`).
    pos: f64,
    last: Option<f32>,
}

impl LinearResampler {
    pub fn new(from_rate: u32, to_rate: u32) -> Self {
        Self {
            from_rate: from_rate.max(1),
            to_rate: to_rate.max(1),
            pos: 0.0,
            last: None,
        }
    }

    pub fn is_passthrough(&self) -> bool {
        self.from_rate == self.to_rate
    }

    pub fn rates(&self) -> (u32, u32) {
        (self.from_rate, self.to_rate)
    }

    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if self.is_passthrough() || input.is_empty() {
            return input.to_vec();
        }
        let step = self.from_rate as f64 / self.to_rate as f64;
        let mut out = Vec::with_capacity((input.len() as f64 / step) as usize + 2);
        // Index -1 is the last sample of the previous chunk.
        let sample_at = |i: isize, last: Option<f32>| -> f32 {
            if i < 0 {
                last.unwrap_or(input[0])
            } else {
                input[i as usize]
            }
        };
        let start = if self.last.is_some() { -1.0 } else { 0.0 };
        let mut pos = self.pos.max(start);
        let n = input.len() as f64;
        while pos <= n - 1.0 {
            let base = pos.floor();
            let frac = (pos - base) as f32;
            let i = base as isize;
            let a = sample_at(i, self.last);
            let b = if (i + 1) as f64 <= n - 1.0 {
                sample_at(i + 1, self.last)
            } else {
                a
            };
            out.push(a + (b - a) * frac);
            pos += step;
        }
        // Carry the position over, measured from the next chunk's first sample.
        self.pos = pos - n;
        self.last = input.last().copied();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(tag: u16, channels: u16, bits: u16, sub: Option<u32>) -> FormatDescription {
        let container = if bits == 24 && tag != FORMAT_TAG_EXTENSIBLE {
            3
        } else {
            (bits / 8).max(1)
        };
        FormatDescription {
            format_tag: tag,
            channels,
            sample_rate: 48_000,
            block_align: container * channels,
            bits_per_sample: bits,
            subformat_tag: sub,
        }
    }

    #[test]
    fn recognises_common_mix_formats() {
        let f = SampleLayout::from_format(&desc(FORMAT_TAG_EXTENSIBLE, 2, 32, Some(3))).unwrap();
        assert_eq!(f.encoding, SampleEncoding::F32);
        assert_eq!(f.channels, 2);
        let f = SampleLayout::from_format(&desc(FORMAT_TAG_PCM, 2, 16, None)).unwrap();
        assert_eq!(f.encoding, SampleEncoding::I16);
        let f = SampleLayout::from_format(&desc(FORMAT_TAG_PCM, 1, 24, None)).unwrap();
        assert_eq!(f.encoding, SampleEncoding::I24Packed);
        let f = SampleLayout::from_format(&desc(FORMAT_TAG_EXTENSIBLE, 6, 32, Some(1))).unwrap();
        assert_eq!(f.encoding, SampleEncoding::I32);
        let f = SampleLayout::from_format(&desc(FORMAT_TAG_IEEE_FLOAT, 2, 64, None)).unwrap();
        assert_eq!(f.encoding, SampleEncoding::F64);
        let f = SampleLayout::from_format(&desc(FORMAT_TAG_PCM, 1, 8, None)).unwrap();
        assert_eq!(f.encoding, SampleEncoding::U8);
    }

    #[test]
    fn rejects_unknown_formats() {
        assert!(SampleLayout::from_format(&desc(0x0055, 2, 16, None)).is_err());
        assert!(SampleLayout::from_format(&desc(FORMAT_TAG_EXTENSIBLE, 2, 32, None)).is_err());
        let mut d = desc(FORMAT_TAG_PCM, 2, 16, None);
        d.channels = 0;
        assert!(SampleLayout::from_format(&d).is_err());
        let mut d = desc(FORMAT_TAG_PCM, 2, 16, None);
        d.sample_rate = 0;
        assert!(SampleLayout::from_format(&d).is_err());
        let mut d = desc(FORMAT_TAG_PCM, 2, 16, None);
        d.block_align = 5;
        assert!(SampleLayout::from_format(&d).is_err());
    }

    #[test]
    fn decodes_stereo_f32_to_mono_average() {
        let layout = SampleLayout::from_format(&desc(FORMAT_TAG_IEEE_FLOAT, 2, 32, None)).unwrap();
        let mut bytes = Vec::new();
        for (l, r) in [(1.0f32, 0.0f32), (0.5, -0.5), (-1.0, -1.0)] {
            bytes.extend_from_slice(&l.to_le_bytes());
            bytes.extend_from_slice(&r.to_le_bytes());
        }
        assert_eq!(layout.decode_mono(&bytes, 3), vec![0.5, 0.0, -1.0]);
        // Asking for more frames than the buffer holds decodes what is there.
        assert_eq!(layout.decode_mono(&bytes, 10).len(), 3);
        assert_eq!(layout.decode_mono(&bytes, 1), vec![0.5]);
    }

    #[test]
    fn decodes_integer_formats() {
        let i16_layout = SampleLayout::from_format(&desc(FORMAT_TAG_PCM, 1, 16, None)).unwrap();
        let mut bytes = Vec::new();
        for v in [i16::MAX, 0, i16::MIN] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let out = i16_layout.decode_mono(&bytes, 3);
        assert!((out[0] - 1.0).abs() < 1e-4);
        assert_eq!(out[1], 0.0);
        assert_eq!(out[2], -1.0);

        let i24 = SampleLayout::from_format(&desc(FORMAT_TAG_PCM, 1, 24, None)).unwrap();
        // -1 in 24-bit two's complement, then +0.5 full scale.
        let bytes = [0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x40];
        let out = i24.decode_mono(&bytes, 2);
        assert!(out[0] < 0.0 && out[0] > -1e-6);
        assert!((out[1] - 0.5).abs() < 1e-6);

        let i32_layout = SampleLayout::from_format(&desc(FORMAT_TAG_PCM, 1, 32, None)).unwrap();
        let bytes = i32::MIN.to_le_bytes();
        assert_eq!(i32_layout.decode_mono(&bytes, 1), vec![-1.0]);

        let u8_layout = SampleLayout::from_format(&desc(FORMAT_TAG_PCM, 1, 8, None)).unwrap();
        assert_eq!(u8_layout.decode_mono(&[128, 0], 2), vec![0.0, -1.0]);
    }

    #[test]
    fn resampler_passthrough_is_identity() {
        let mut r = LinearResampler::new(48_000, 48_000);
        assert!(r.is_passthrough());
        assert_eq!(r.process(&[0.1, 0.2]), vec![0.1, 0.2]);
    }

    #[test]
    fn resampler_keeps_duration_across_chunks() {
        // One second of 44.1 kHz in 441-sample chunks becomes ~one second of
        // 48 kHz, whatever the chunking.
        let mut r = LinearResampler::new(44_100, 48_000);
        let chunk: Vec<f32> = (0..441).map(|i| (i as f32 * 0.01).sin()).collect();
        let mut total = 0usize;
        for _ in 0..100 {
            total += r.process(&chunk).len();
        }
        assert!((total as i64 - 48_000).abs() <= 2, "got {total}");

        let mut down = LinearResampler::new(48_000, 16_000);
        let chunk = vec![0.0f32; 480];
        let mut total = 0usize;
        for _ in 0..100 {
            total += down.process(&chunk).len();
        }
        assert!((total as i64 - 16_000).abs() <= 2, "got {total}");
    }

    #[test]
    fn resampler_interpolates_a_ramp() {
        let mut r = LinearResampler::new(1, 2);
        let out = r.process(&[0.0, 1.0, 2.0]);
        assert_eq!(out, vec![0.0, 0.5, 1.0, 1.5, 2.0]);
        // The next chunk continues between the previous last sample and its
        // own first sample.
        let out = r.process(&[3.0]);
        assert_eq!(out, vec![2.5, 3.0]);
    }
}
