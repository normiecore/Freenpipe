// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Decode one time window of an audio file with the built-in symphonia
//! decoder (W2-16, audio-005), for callers that only need a short clip and
//! should not pay for the whole file.

use std::path::Path;

use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{Decoder, CODEC_TYPE_NULL};
use symphonia::core::conv::FromSample;
use symphonia::core::errors::{Error as SymphoniaError, SeekErrorKind};
use symphonia::core::formats::{FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::units::{Time, TimeBase, TimeStamp};
use tracing::debug;

/// An opened file positioned on its first decodable audio track.
struct OpenedTrack {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    sample_rate: u32,
    time_base: Option<TimeBase>,
}

impl OpenedTrack {
    fn open(path: &Path) -> anyhow::Result<Self> {
        let file = std::fs::File::open(path)?;
        let stream = MediaSourceStream::new(Box::new(file), Default::default());
        let probed = symphonia::default::get_probe().format(
            &Default::default(),
            stream,
            &Default::default(),
            &Default::default(),
        )?;
        let format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| anyhow::anyhow!("no supported audio tracks found"))?;
        let sample_rate = track
            .codec_params
            .sample_rate
            .filter(|rate| *rate > 0)
            .ok_or_else(|| anyhow::anyhow!("could not determine sample rate"))?;
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &Default::default())
            .map_err(|_| anyhow::anyhow!("unsupported codec"))?;
        Ok(Self {
            track_id: track.id,
            sample_rate,
            time_base: track.codec_params.time_base,
            format,
            decoder,
        })
    }

    /// Frame index (at the native sample rate) of a packet timestamp.
    fn frame_at(&self, ts: TimeStamp) -> u64 {
        match self.time_base {
            Some(time_base) if time_base.numer > 0 && time_base.denom > 0 => {
                let time = time_base.calc_time(ts);
                let seconds = time.seconds as f64 + time.frac;
                (seconds * f64::from(self.sample_rate)).round() as u64
            }
            // Without a time base, packet timestamps count frames.
            _ => ts,
        }
    }
}

/// Append the first channel of a decoded buffer to `out`.
fn push_first_channel(decoded: AudioBufferRef<'_>, out: &mut Vec<f32>) {
    fn convert<T>(buffer: &symphonia::core::audio::AudioBuffer<T>, out: &mut Vec<f32>)
    where
        T: symphonia::core::sample::Sample,
        f32: FromSample<T>,
    {
        out.extend(buffer.chan(0).iter().map(|v| f32::from_sample(*v)));
    }
    match decoded {
        AudioBufferRef::F32(buffer) => out.extend_from_slice(buffer.chan(0)),
        AudioBufferRef::U8(buffer) => convert(&buffer, out),
        AudioBufferRef::U16(buffer) => convert(&buffer, out),
        AudioBufferRef::U24(buffer) => convert(&buffer, out),
        AudioBufferRef::U32(buffer) => convert(&buffer, out),
        AudioBufferRef::S8(buffer) => convert(&buffer, out),
        AudioBufferRef::S16(buffer) => convert(&buffer, out),
        AudioBufferRef::S24(buffer) => convert(&buffer, out),
        AudioBufferRef::S32(buffer) => convert(&buffer, out),
        AudioBufferRef::F64(buffer) => convert(&buffer, out),
    }
}

/// Decode packets from the reader's current position and keep the frames in
/// `[first, last)` (native-rate frame indices).
fn collect_frames(track: &mut OpenedTrack, first: u64, last: u64) -> anyhow::Result<Vec<f32>> {
    let capacity = usize::try_from(last.saturating_sub(first)).unwrap_or(0);
    let mut out = Vec::with_capacity(capacity.min(16_000 * 300));
    let mut decoded_frames = Vec::new();
    loop {
        let packet = match track.format.next_packet() {
            Ok(packet) => packet,
            // Symphonia reports the end of the stream as an unexpected EOF.
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break
            }
            // A chained stream would need a new decoder; the window ends here.
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(e.into()),
        };
        while !track.format.metadata().is_latest() {
            track.format.metadata().pop();
        }
        if packet.track_id() != track.track_id {
            continue;
        }
        let packet_first = track.frame_at(packet.ts());
        if packet_first >= last {
            break;
        }
        let decoded = match track.decoder.decode(&packet) {
            Ok(decoded) => decoded,
            // One bad packet should not fail the clip; symphonia recommends
            // skipping it.
            Err(SymphoniaError::DecodeError(reason)) => {
                debug!("pcm window: skipping undecodable packet: {}", reason);
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        decoded_frames.clear();
        push_first_channel(decoded, &mut decoded_frames);

        let keep_from = usize::try_from(first.saturating_sub(packet_first)).unwrap_or(usize::MAX);
        let keep_to = usize::try_from(last - packet_first)
            .unwrap_or(usize::MAX)
            .min(decoded_frames.len());
        if keep_from < keep_to {
            out.extend_from_slice(&decoded_frames[keep_from..keep_to]);
        }
    }
    Ok(out)
}

/// Decode `duration_secs` of audio that starts `start_secs` into the file
/// (first channel, native sample rate).
///
/// Returns `(samples, sample_rate)`, where the first sample sits at
/// `start_secs`. The buffer is empty when the window starts at or after the
/// end of the file, and shorter than asked when the file ends inside the
/// window. The reader seeks when the container supports it; otherwise the file
/// is decoded from the start and only the window is kept, so memory stays
/// bounded by the window either way. Files that cannot be opened or decoded
/// are errors.
pub fn pcm_decode_window<P: AsRef<Path>>(
    path: P,
    start_secs: f64,
    duration_secs: f64,
) -> anyhow::Result<(Vec<f32>, u32)> {
    if !start_secs.is_finite() || start_secs < 0.0 {
        anyhow::bail!("window start must be a finite, non-negative number of seconds");
    }
    if !duration_secs.is_finite() || duration_secs <= 0.0 {
        anyhow::bail!("window length must be a finite, positive number of seconds");
    }
    let path = path.as_ref();
    let mut track = OpenedTrack::open(path)?;
    let rate = f64::from(track.sample_rate);
    let first = (start_secs * rate).floor() as u64;
    let last = ((start_secs + duration_secs) * rate).ceil() as u64;

    if first > 0 {
        let target = SeekTo::Time {
            time: Time::from(start_secs),
            track_id: Some(track.track_id),
        };
        match track.format.seek(SeekMode::Accurate, target) {
            Ok(_) => track.decoder.reset(),
            Err(SymphoniaError::SeekError(SeekErrorKind::OutOfRange)) => {
                return Ok((Vec::new(), track.sample_rate));
            }
            Err(e) => {
                // Not every container can seek; read from the start instead.
                debug!(
                    "pcm window: seek in {} failed ({}), decoding from the start",
                    path.display(),
                    e
                );
                track = OpenedTrack::open(path)?;
            }
        }
    }

    let samples = collect_frames(&mut track, first, last)?;
    Ok((samples, track.sample_rate))
}
