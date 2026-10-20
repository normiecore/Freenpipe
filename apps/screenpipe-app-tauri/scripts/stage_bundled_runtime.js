// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

// Stages the runtime files the Windows installer ships next to the app, under
// src-tauri/bundled/ (mapped as bundle resources in tauri.windows.conf.json):
//
//   bundled/cli/screenpipe(.exe)          -> <install>/screenpipe(.exe)
//   bundled/mcp/screenpipe-mcp.js         -> <install>/mcp/screenpipe-mcp.js
//   bundled/models/pyannote/<model>.onnx  -> <install>/models/pyannote/
//   bundled/models/pyannote/NOTICE.md     (sources and licences of the models)
//   bundled/vad/silero_vad_v5.onnx        -> <install>/vad/ (with NOTICE.md)
//   bundled/runtime/*                     -> <install>/   (Windows only)
//
// bundled/runtime holds the native runtime the GPU paths need, each with its
// licence text next to it:
//   - onnxruntime.dll + onnxruntime_providers_shared.dll: ONNX Runtime 1.22.0
//     built with the DirectML provider (MIT, NuGet), which also contains the
//     CPU provider and loads DirectML.dll only when DirectML is used;
//   - vulkan-1.dll + VulkanRT-License.txt: the Vulkan loader whisper links
//     against in builds with the `vulkan` feature (Apache-2.0), copied from
//     FREENPIPE_VULKAN_RUNTIME_DIR;
//   - DirectML.dll: Microsoft's DirectML redistributable, only with
//     FREENPIPE_BUNDLE_DIRECTML=1. It ships under Microsoft's own licence
//     terms, not an open-source licence, so it stays out of builds until the
//     release licence audit clears it (owner decision 7); without it
//     Parakeet runs on CPU unless SCREENPIPE_DIRECTML=1 allows the copy that
//     Windows itself provides (transcription/parakeet_windows.rs).
//
// Nothing here resolves a package from a registry at app run time: the MCP
// server is bundled from packages/screenpipe-mcp in this repository, the CLI
// is the engine binary built from this repository, and the speaker models
// come from the repository checkout (when Git LFS content is present) or from
// the repository's LFS storage at a pinned commit, checked against pinned
// SHA-256 values.
//
// Each step is best effort so a plain `bun run dev` keeps working offline; the
// app then falls back at run time (REST API instead of the CLI, model
// download on first use). Release builds set FREENPIPE_REQUIRE_BUNDLED_RUNTIME=1
// so a missing piece fails the build instead.
//
// Environment:
//   FREENPIPE_CLI_BINARY=<path>        copy this prebuilt CLI binary
//   FREENPIPE_BUILD_CLI=1              build it: cargo build --release --bin screenpipe
//   FREENPIPE_CLI_FEATURES=<features>  extra cargo features for that build
//   SCREENPIPE_PYANNOTE_BASE_URL=<url> model mirror (same variable the app reads)
//   FREENPIPE_VULKAN_RUNTIME_DIR=<dir>   folder holding vulkan-1.dll and its licence
//   FREENPIPE_BUNDLE_DIRECTML=1        also ship DirectML.dll (licence review first)
//   FREENPIPE_REQUIRE_BUNDLED_RUNTIME=1  make every step mandatory

import { $ } from 'bun'
import { createHash } from 'crypto'
import fs from 'fs/promises'
import path from 'path'
import { inflateRawSync } from 'zlib'

// GitHub's LFS media endpoint for crates/screenpipe-audio/models/pyannote/,
// addressed by the import commit so the files cannot change.
export const DEFAULT_PYANNOTE_BASE_URL =
	'https://media.githubusercontent.com/media/normiecore/Freenpipe/bcef0bcde5e6ea12db811a27047ef63869b7374a/crates/screenpipe-audio/models/pyannote'

// Same values as crates/screenpipe-audio/src/speaker/models.rs (a test keeps
// them in sync). Size and hash come from the repository's Git LFS pointers;
// `asset` is the file name as it appears in the mirror URL (percent-encoded).
export const PYANNOTE_MODELS = [
	{
		filename: 'segmentation-3.0.onnx',
		asset: 'segmentation-3.0.onnx',
		size: 5983836,
		sha256: 'b78fc48113bb46fd247ae6a9aea737079550c647638db961df7e0e1e9f4ba62e',
	},
	{
		filename: 'wespeaker_en_voxceleb_CAM++.onnx',
		asset: 'wespeaker_en_voxceleb_CAM%2B%2B.onnx',
		size: 29292684,
		sha256: 'c46fad10b5f81e1aa4a60c162714208577093655076c5450f8c469e522ec54ef',
	},
]

// Silero VAD v5.1.2 (snakers4/silero-vad, MIT). Same values as
// crates/screenpipe-audio/src/vad/silero.rs (a test keeps them in sync).
export const SILERO_MODEL = {
	filename: 'silero_vad_v5.onnx',
	url: 'https://raw.githubusercontent.com/snakers4/silero-vad/6478567951ae5c9979ad7b234185b5515f4be7a1/src/silero_vad/data/silero_vad.onnx',
	size: 2327524,
	sha256: '2623a2953f6ff3d2c1e61740c6cdb7168133479b267dfef114a4a3cc5bdd788f',
}

/**
 * NuGet packages the Windows runtime comes from, pinned by size and SHA-256
 * of the .nupkg. `files(arch)` lists what is copied out of each, for 'x64'
 * or 'arm64', as { entry: path in the package, dest: name in bundled/runtime }.
 */
export const ORT_DIRECTML_PACKAGE = {
	label: 'ONNX Runtime 1.22.0 (DirectML build, MIT)',
	url: 'https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime.directml/1.22.0/microsoft.ml.onnxruntime.directml.1.22.0.nupkg',
	size: 17898472,
	sha256: '29f9872d786236b79aa83f94482f3a17c14297e4833768d6d0ed4883ee732e60',
	files: (arch) => [
		{ entry: `runtimes/win-${arch}/native/onnxruntime.dll`, dest: 'onnxruntime.dll', dll: true },
		{
			entry: `runtimes/win-${arch}/native/onnxruntime_providers_shared.dll`,
			dest: 'onnxruntime_providers_shared.dll',
			dll: true,
		},
		{ entry: 'LICENSE', dest: 'onnxruntime-LICENSE.txt' },
		{ entry: 'ThirdPartyNotices.txt', dest: 'onnxruntime-ThirdPartyNotices.txt' },
	],
}

export const DIRECTML_PACKAGE = {
	label: 'DirectML 1.15.4 redistributable (Microsoft licence terms)',
	url: 'https://api.nuget.org/v3-flatcontainer/microsoft.ai.directml/1.15.4/microsoft.ai.directml.1.15.4.nupkg',
	size: 202292617,
	sha256: '4e7cb7ddce8cf837a7a75dc029209b520ca0101470fcdf275c1f49736a3615b9',
	files: (arch) => [
		{ entry: `bin/${arch}-win/DirectML.dll`, dest: 'DirectML.dll', dll: true },
		{ entry: 'LICENSE.txt', dest: 'DirectML-License.txt' },
		{ entry: 'ThirdPartyNotices.txt', dest: 'DirectML-ThirdPartyNotices.txt' },
	],
}

/** File names the Vulkan loader is staged under (same names as screenpipe's install). */
export const VULKAN_LOADER = { dll: 'vulkan-1.dll', licence: 'VulkanRT-License.txt' }

/** PE machine types of the Windows architectures Freenpipe ships. */
export const PE_MACHINE = { x64: 0x8664, arm64: 0xaa64 }

export const MCP_ENTRY = path.join('mcp', 'screenpipe-mcp.js')

/** Attribution file staged next to the models (from the repository folder). */
export const MODEL_NOTICE = 'NOTICE.md'

// Upper bound for one model download, so a stalled connection cannot hang
// the build (the larger file is about 30 MB).
export const MODEL_FETCH_TIMEOUT_MS = 10 * 60 * 1000

// Text that must never appear in the shipped MCP bundle: registry packages
// resolved at run time, and hosts of the upstream vendor's services.
const FORBIDDEN_IN_MCP_BUNDLE = [
	[/screenpipe(-mcp)?@latest/, 'a registry package reference'],
	[/screenpi\.pe|screenpipe\.com|mediar\.ai/i, 'a vendor service host'],
]

/** Throws when the built MCP bundle text contains anything forbidden. */
export function assertCleanMcpBundle(text) {
	for (const [pattern, what] of FORBIDDEN_IN_MCP_BUNDLE) {
		const hit = text.match(pattern)
		if (hit) throw new Error(`MCP bundle contains ${what} (${hit[0]})`)
	}
}

/** Mirror URL of one model. Rejects anything but http(s) bases. */
export function pyannoteModelUrl(model, baseOverride) {
	const raw = (baseOverride ?? '').trim()
	const base = /^https?:\/\//.test(raw) ? raw.replace(/\/+$/, '') : DEFAULT_PYANNOTE_BASE_URL
	return `${base}/${model.asset}`
}

export function sha256Hex(buffer) {
	return createHash('sha256').update(buffer).digest('hex')
}

/** True when `buffer` is exactly the pinned model. */
export function isExpectedModel(buffer, model) {
	return buffer.length === model.size && sha256Hex(buffer) === model.sha256
}

async function readIfSized(file, size) {
	try {
		const stat = await fs.stat(file)
		if (!stat.isFile() || stat.size !== size) return null
		return await fs.readFile(file)
	} catch {
		return null
	}
}

async function writeAtomically(dest, buffer) {
	const tmp = `${dest}.partial`
	await fs.writeFile(tmp, buffer)
	await fs.rename(tmp, dest)
}

/** Windows architecture of the build: from SCREENPIPE_RELEASE_TARGET, else this machine. */
export function windowsArch(releaseTarget = process.env.SCREENPIPE_RELEASE_TARGET, hostArch = process.arch) {
	if (releaseTarget?.startsWith('aarch64')) return 'arm64'
	if (releaseTarget?.startsWith('x86_64')) return 'x64'
	return hostArch === 'arm64' ? 'arm64' : 'x64'
}

/** Machine field of a PE image (0x8664 x64, 0xaa64 arm64), or null when `buffer` is not one. */
export function peMachine(buffer) {
	if (buffer.length < 0x40 || buffer.readUInt16LE(0) !== 0x5a4d) return null // "MZ"
	const pe = buffer.readUInt32LE(0x3c)
	if (pe + 6 > buffer.length || buffer.readUInt32LE(pe) !== 0x00004550) return null // "PE\0\0"
	return buffer.readUInt16LE(pe + 4)
}

/** Throws unless `buffer` is a DLL built for `arch`. */
export function assertDllForArch(buffer, arch, name) {
	const machine = peMachine(buffer)
	if (machine === null) throw new Error(`${name} is not a Windows PE image`)
	if (machine !== PE_MACHINE[arch]) {
		throw new Error(`${name} is built for machine 0x${machine.toString(16)}, not ${arch}`)
	}
}

function findEndOfCentralDirectory(buffer) {
	const lowest = Math.max(0, buffer.length - 0xffff - 22)
	for (let i = buffer.length - 22; i >= lowest; i--) {
		if (buffer.readUInt32LE(i) === 0x06054b50) return i
	}
	throw new Error('not a zip archive (no end-of-central-directory record)')
}

/**
 * Extract the named entries of a zip archive (a .nupkg is one) held in
 * memory. Supports stored and deflated entries, which is what NuGet packages
 * use; returns Map<name, Buffer> and throws when a name is missing.
 */
export function readZipEntries(buffer, names) {
	const wanted = new Set(names)
	const end = findEndOfCentralDirectory(buffer)
	const count = buffer.readUInt16LE(end + 10)
	let offset = buffer.readUInt32LE(end + 16)
	const found = new Map()
	for (let i = 0; i < count; i++) {
		if (buffer.readUInt32LE(offset) !== 0x02014b50) throw new Error('corrupt zip central directory')
		const method = buffer.readUInt16LE(offset + 10)
		const compressedSize = buffer.readUInt32LE(offset + 20)
		const size = buffer.readUInt32LE(offset + 24)
		const nameLength = buffer.readUInt16LE(offset + 28)
		const extraLength = buffer.readUInt16LE(offset + 30)
		const commentLength = buffer.readUInt16LE(offset + 32)
		const localHeader = buffer.readUInt32LE(offset + 42)
		const name = buffer.toString('utf8', offset + 46, offset + 46 + nameLength)
		offset += 46 + nameLength + extraLength + commentLength
		if (!wanted.has(name)) continue
		if ([compressedSize, size, localHeader].includes(0xffffffff)) {
			throw new Error(`${name}: zip64 entries are not supported`)
		}
		if (buffer.readUInt32LE(localHeader) !== 0x04034b50) throw new Error(`${name}: corrupt local header`)
		const start = localHeader + 30 + buffer.readUInt16LE(localHeader + 26) + buffer.readUInt16LE(localHeader + 28)
		const raw = buffer.subarray(start, start + compressedSize)
		let data
		if (method === 0) data = Buffer.from(raw)
		else if (method === 8) data = inflateRawSync(raw)
		else throw new Error(`${name}: unsupported zip compression method ${method}`)
		if (data.length !== size) throw new Error(`${name}: extracted ${data.length} bytes, expected ${size}`)
		found.set(name, data)
	}
	for (const name of wanted) {
		if (!found.has(name)) throw new Error(`${name} is not in the archive`)
	}
	return found
}

/**
 * Pick the Vulkan loader for `arch` and a licence file from a list of paths
 * relative to FREENPIPE_VULKAN_RUNTIME_DIR ('/'-separated). The loader is the
 * `vulkan-1.dll` in an architecture folder (x64/amd64/x86_64/win64 or
 * arm64/aarch64), or one at the top of the folder; the licence is a
 * *licen[cs]e* text file, the shallowest one first.
 */
export function pickVulkanRuntimeFiles(relativePaths, arch) {
	const archDirs = arch === 'arm64' ? ['arm64', 'aarch64'] : ['x64', 'amd64', 'x86_64', 'win64']
	const parts = (p) => p.split('/').filter(Boolean)
	const loaders = relativePaths.filter((p) => parts(p).at(-1)?.toLowerCase() === VULKAN_LOADER.dll)
	const inArchDir = loaders.find((p) => {
		const segs = parts(p)
		return segs.length >= 2 && archDirs.includes(segs.at(-2).toLowerCase())
	})
	const atTop = loaders.find((p) => parts(p).length === 1)
	const dll = inArchDir ?? atTop
	if (!dll) {
		throw new Error(
			loaders.length
				? `no ${VULKAN_LOADER.dll} for ${arch} (found: ${loaders.join(', ')})`
				: `no ${VULKAN_LOADER.dll} in the folder`,
		)
	}
	const licence = relativePaths
		.filter((p) => /licen[cs]e/i.test(parts(p).at(-1) ?? '') && /(\.txt|\.md|^[^.]*)$/i.test(parts(p).at(-1)))
		.sort((a, b) => parts(a).length - parts(b).length || a.localeCompare(b))[0]
	if (!licence) throw new Error('no licence file next to the Vulkan loader; refusing to ship it without one')
	return { dll, licence }
}

/**
 * The pinned package, from bundled/.cache when an earlier run left a verified
 * copy there, otherwise downloaded and verified (then cached).
 */
async function fetchPinnedPackage(pkg, stageRoot) {
	const cacheDir = path.join(stageRoot, '.cache')
	const cached = path.join(cacheDir, path.basename(new URL(pkg.url).pathname))
	const existing = await readIfSized(cached, pkg.size)
	if (existing && isExpectedModel(existing, pkg)) return existing
	const res = await fetch(pkg.url, { signal: AbortSignal.timeout(MODEL_FETCH_TIMEOUT_MS) })
	if (!res.ok) throw new Error(`HTTP ${res.status} from ${pkg.url}`)
	const body = Buffer.from(await res.arrayBuffer())
	if (!isExpectedModel(body, pkg)) {
		throw new Error(`${pkg.url} does not match the pinned size/sha256`)
	}
	await fs.mkdir(cacheDir, { recursive: true })
	await writeAtomically(cached, body)
	return body
}

/** The listed files of a pinned package, each DLL checked against `arch`. */
export function extractPackageFiles(packageBuffer, files, arch) {
	const entries = readZipEntries(
		packageBuffer,
		files.map((f) => f.entry),
	)
	return files.map((f) => {
		const data = entries.get(f.entry)
		if (f.dll) assertDllForArch(data, arch, f.entry)
		return { dest: f.dest, data }
	})
}

async function writeRuntimeFiles(destDir, extracted) {
	for (const { dest, data } of extracted) {
		await writeAtomically(path.join(destDir, dest), data)
	}
}

/**
 * Bundle packages/screenpipe-mcp into one CommonJS file (dependencies
 * inlined) that runs under the bundled bun, or node, with no install step.
 */
export async function buildMcpBundle(repoRoot, outFile) {
	const pkgDir = path.join(repoRoot, 'packages', 'screenpipe-mcp')
	// --frozen-lockfile: build exactly the versions pinned in bun.lock, and
	// fail instead of rewriting the lockfile when package.json drifted.
	await $`bun install --frozen-lockfile`.cwd(pkgDir)
	await $`bun build ./src/cli.ts --target=node --format=cjs --outfile=${outFile}`.cwd(pkgDir)
	try {
		assertCleanMcpBundle(await fs.readFile(outFile, 'utf8'))
	} catch (error) {
		await fs.rm(outFile, { force: true })
		throw error
	}
	return outFile
}

async function stageMcp({ repoRoot, stageRoot }) {
	const out = await buildMcpBundle(repoRoot, path.join(stageRoot, MCP_ENTRY))
	console.log(`[bundled-runtime] MCP server bundled at ${path.relative(repoRoot, out)}`)
}

async function stageCli({ repoRoot, stageRoot, platform }) {
	const exe = platform === 'windows' ? 'screenpipe.exe' : 'screenpipe'
	const dest = path.join(stageRoot, 'cli', exe)
	let source = process.env.FREENPIPE_CLI_BINARY
	if (!source && process.env.FREENPIPE_BUILD_CLI === '1') {
		const features = process.env.FREENPIPE_CLI_FEATURES
		const featureArgs = features ? ['--features', features] : []
		await $`cargo build --release -p screenpipe-engine --bin screenpipe ${featureArgs}`.cwd(repoRoot)
		const targetDir = process.env.CARGO_TARGET_DIR || path.join(repoRoot, 'target')
		source = path.join(targetDir, 'release', exe)
	}
	if (!source) {
		// A release must ship the CLI built for it, never one left over from
		// an earlier run, so required mode ignores a previously staged copy.
		if (process.env.FREENPIPE_REQUIRE_BUNDLED_RUNTIME === '1') {
			await fs.rm(dest, { force: true })
			throw new Error('no CLI binary: set FREENPIPE_CLI_BINARY or FREENPIPE_BUILD_CLI=1')
		}
		const staged = await fs.stat(dest).then(() => true, () => false)
		const note = staged ? 'keeping the copy staged earlier' : 'agents will use the REST API instead'
		console.log(`[bundled-runtime] CLI not provided (FREENPIPE_CLI_BINARY / FREENPIPE_BUILD_CLI); ${note}`)
		return
	}
	await fs.copyFile(source, dest)
	if (platform !== 'windows') await fs.chmod(dest, 0o755)
	console.log(`[bundled-runtime] CLI staged from ${source}`)
}

async function stagePyannote({ repoRoot, stageRoot }) {
	const destDir = path.join(stageRoot, 'models', 'pyannote')
	const repoDir = path.join(repoRoot, 'crates', 'screenpipe-audio', 'models', 'pyannote')
	// Sources and licences of the weights ship next to them.
	await fs.copyFile(path.join(repoDir, MODEL_NOTICE), path.join(destDir, MODEL_NOTICE))
	for (const model of PYANNOTE_MODELS) {
		const dest = path.join(destDir, model.filename)
		const staged = await readIfSized(dest, model.size)
		if (staged && isExpectedModel(staged, model)) continue

		const repoCopy = await readIfSized(path.join(repoDir, model.filename), model.size)
		if (repoCopy && isExpectedModel(repoCopy, model)) {
			await writeAtomically(dest, repoCopy)
			console.log(`[bundled-runtime] ${model.filename} staged from the repository`)
			continue
		}

		const url = pyannoteModelUrl(model, process.env.SCREENPIPE_PYANNOTE_BASE_URL)
		const res = await fetch(url, { signal: AbortSignal.timeout(MODEL_FETCH_TIMEOUT_MS) })
		if (!res.ok) throw new Error(`${model.filename}: HTTP ${res.status} from ${url}`)
		const body = Buffer.from(await res.arrayBuffer())
		if (!isExpectedModel(body, model)) {
			throw new Error(`${model.filename} from ${url} does not match the pinned size/sha256`)
		}
		await writeAtomically(dest, body)
		console.log(`[bundled-runtime] ${model.filename} downloaded from ${url}`)
	}
}

async function stageSilero({ repoRoot, stageRoot }) {
	const destDir = path.join(stageRoot, 'vad')
	const noticeDir = path.join(repoRoot, 'crates', 'screenpipe-audio', 'models', 'silero')
	await fs.copyFile(path.join(noticeDir, MODEL_NOTICE), path.join(destDir, MODEL_NOTICE))
	const dest = path.join(destDir, SILERO_MODEL.filename)
	const staged = await readIfSized(dest, SILERO_MODEL.size)
	if (staged && isExpectedModel(staged, SILERO_MODEL)) return
	const res = await fetch(SILERO_MODEL.url, { signal: AbortSignal.timeout(MODEL_FETCH_TIMEOUT_MS) })
	if (!res.ok) throw new Error(`HTTP ${res.status} from ${SILERO_MODEL.url}`)
	const body = Buffer.from(await res.arrayBuffer())
	if (!isExpectedModel(body, SILERO_MODEL)) {
		throw new Error(`${SILERO_MODEL.url} does not match the pinned size/sha256`)
	}
	await writeAtomically(dest, body)
	console.log(`[bundled-runtime] ${SILERO_MODEL.filename} downloaded from ${SILERO_MODEL.url}`)
}

/**
 * ONNX Runtime with the DirectML provider. When the package cannot be
 * fetched outside required mode, a CPU build already unpacked under
 * src-tauri/onnxruntime-win-<arch>-<version>/lib is staged instead, so a dev build
 * still ships a working runtime.
 */
async function stageOnnxRuntime({ srcTauriDir, stageRoot, platform, arch, required }) {
	if (platform !== 'windows') return
	const destDir = path.join(stageRoot, 'runtime')
	try {
		const pkg = await fetchPinnedPackage(ORT_DIRECTML_PACKAGE, stageRoot)
		await writeRuntimeFiles(destDir, extractPackageFiles(pkg, ORT_DIRECTML_PACKAGE.files(arch), arch))
		console.log(`[bundled-runtime] ${ORT_DIRECTML_PACKAGE.label} staged for ${arch}`)
	} catch (error) {
		if (required) throw error
		const entries = await fs.readdir(srcTauriDir).catch(() => [])
		const cpuDir = entries.filter((d) => d.startsWith(`onnxruntime-win-${arch}-`)).sort().at(-1)
		if (!cpuDir) throw error
		for (const name of ['onnxruntime.dll', 'onnxruntime_providers_shared.dll']) {
			await fs.copyFile(path.join(srcTauriDir, cpuDir, 'lib', name), path.join(destDir, name))
		}
		// The release zip carries the same licence texts at its top level.
		for (const [name, dest] of [
			['LICENSE', 'onnxruntime-LICENSE.txt'],
			['ThirdPartyNotices.txt', 'onnxruntime-ThirdPartyNotices.txt'],
		]) {
			await fs.copyFile(path.join(srcTauriDir, cpuDir, name), path.join(destDir, dest)).catch(() => {})
		}
		console.warn(
			`[bundled-runtime] DirectML build of ONNX Runtime not staged (${error.message}); using the CPU build in ${cpuDir}`,
		)
	}
}

/**
 * DirectML.dll, only on request: its licence terms are Microsoft's own and
 * need the release licence audit (decision 7). Without the request any copy
 * left by an earlier run is removed, so it cannot ship by accident.
 */
export async function stageDirectMl({ stageRoot, platform, arch }) {
	if (platform !== 'windows') return
	const destDir = path.join(stageRoot, 'runtime')
	if (process.env.FREENPIPE_BUNDLE_DIRECTML !== '1') {
		for (const f of DIRECTML_PACKAGE.files(arch)) await fs.rm(path.join(destDir, f.dest), { force: true })
		console.log(
			'[bundled-runtime] DirectML.dll not bundled (FREENPIPE_BUNDLE_DIRECTML=1 after the licence review); Parakeet uses CPU unless SCREENPIPE_DIRECTML=1',
		)
		return
	}
	const pkg = await fetchPinnedPackage(DIRECTML_PACKAGE, stageRoot)
	await writeRuntimeFiles(destDir, extractPackageFiles(pkg, DIRECTML_PACKAGE.files(arch), arch))
	console.log(`[bundled-runtime] ${DIRECTML_PACKAGE.label} staged for ${arch}`)
}

async function listFilesRelative(root) {
	const entries = await fs.readdir(root, { recursive: true, withFileTypes: true })
	return entries
		.filter((e) => e.isFile())
		.map((e) => path.relative(root, path.join(e.parentPath ?? e.path, e.name)).split(path.sep).join('/'))
}

/** The Vulkan loader and its licence, from FREENPIPE_VULKAN_RUNTIME_DIR. */
export async function stageVulkanLoader({ stageRoot, platform, arch }) {
	if (platform !== 'windows') return
	const source = process.env.FREENPIPE_VULKAN_RUNTIME_DIR
	if (!source) {
		console.log(
			'[bundled-runtime] FREENPIPE_VULKAN_RUNTIME_DIR not set; vulkan-1.dll not staged (builds with the `vulkan` feature need it next to the exe)',
		)
		return
	}
	const picked = pickVulkanRuntimeFiles(await listFilesRelative(source), arch)
	const dll = await fs.readFile(path.join(source, picked.dll))
	assertDllForArch(dll, arch, picked.dll)
	const destDir = path.join(stageRoot, 'runtime')
	await writeAtomically(path.join(destDir, VULKAN_LOADER.dll), dll)
	await fs.copyFile(path.join(source, picked.licence), path.join(destDir, VULKAN_LOADER.licence))
	console.log(
		`[bundled-runtime] Vulkan loader staged from ${picked.dll} (sha256 ${sha256Hex(dll)}), licence from ${picked.licence}`,
	)
}

/**
 * Stage every bundled runtime piece. `srcTauriDir` is apps/.../src-tauri,
 * `platform` is 'windows' | 'macos' | 'linux'; `arch` ('x64' | 'arm64') is
 * the Windows architecture, by default from SCREENPIPE_RELEASE_TARGET.
 */
export async function stageBundledRuntime({ srcTauriDir, platform, arch = windowsArch() }) {
	const repoRoot = path.resolve(srcTauriDir, '..', '..', '..')
	const stageRoot = path.join(srcTauriDir, 'bundled')
	// The resource mappings point at these directories, so they must exist
	// even when a step is skipped (an empty directory bundles nothing).
	for (const dir of ['cli', 'mcp', path.join('models', 'pyannote'), 'vad', 'runtime']) {
		await fs.mkdir(path.join(stageRoot, dir), { recursive: true })
	}
	const required = process.env.FREENPIPE_REQUIRE_BUNDLED_RUNTIME === '1'
	const steps = [
		['MCP server', stageMcp],
		['CLI', stageCli],
		['pyannote models', stagePyannote],
		['silero VAD model', stageSilero],
		['ONNX Runtime', stageOnnxRuntime],
		['DirectML', stageDirectMl],
		['Vulkan loader', stageVulkanLoader],
	]
	for (const [label, step] of steps) {
		try {
			await step({ repoRoot, srcTauriDir, stageRoot, platform, arch, required })
		} catch (error) {
			if (required) throw new Error(`[bundled-runtime] ${label}: ${error.message}`)
			console.warn(`[bundled-runtime] ${label} not staged: ${error.message}`)
		}
	}
}
