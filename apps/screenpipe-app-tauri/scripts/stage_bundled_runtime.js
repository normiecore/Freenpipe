// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

// Stages the runtime files the Windows installer ships next to the app, under
// src-tauri/bundled/ (mapped as bundle resources in tauri.windows.conf.json):
//
//   bundled/cli/screenpipe(.exe)          -> <install>/screenpipe(.exe)
//   bundled/mcp/screenpipe-mcp.js         -> <install>/mcp/screenpipe-mcp.js
//   bundled/models/pyannote/<model>.onnx  -> <install>/models/pyannote/
//
// Nothing here resolves a package from a registry at app run time: the MCP
// server is bundled from packages/screenpipe-mcp in this repository, the CLI
// is the engine binary built from this repository, and the speaker models
// come from the repository (Git LFS) or the Freenpipe release mirror, checked
// against pinned SHA-256 values.
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
//   FREENPIPE_REQUIRE_BUNDLED_RUNTIME=1  make every step mandatory

import { $ } from 'bun'
import { createHash } from 'crypto'
import fs from 'fs/promises'
import path from 'path'

export const DEFAULT_PYANNOTE_BASE_URL =
	'https://github.com/normiecore/Freenpipe/releases/download/pyannote-models-v1'

// Same values as crates/screenpipe-audio/src/speaker/models.rs (a test keeps
// them in sync). Size and hash come from the repository's Git LFS pointers.
export const PYANNOTE_MODELS = [
	{
		filename: 'segmentation-3.0.onnx',
		asset: 'pyannote-segmentation-3.0.onnx',
		size: 5983836,
		sha256: 'b78fc48113bb46fd247ae6a9aea737079550c647638db961df7e0e1e9f4ba62e',
	},
	{
		filename: 'wespeaker_en_voxceleb_CAM++.onnx',
		asset: 'wespeaker-en-voxceleb-campp.onnx',
		size: 29292684,
		sha256: 'c46fad10b5f81e1aa4a60c162714208577093655076c5450f8c469e522ec54ef',
	},
]

export const MCP_ENTRY = path.join('mcp', 'screenpipe-mcp.js')

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

/**
 * Bundle packages/screenpipe-mcp into one CommonJS file (dependencies
 * inlined) that runs under the bundled bun, or node, with no install step.
 */
export async function buildMcpBundle(repoRoot, outFile) {
	const pkgDir = path.join(repoRoot, 'packages', 'screenpipe-mcp')
	await $`bun install`.cwd(pkgDir)
	await $`bun build ./src/cli.ts --target=node --format=cjs --outfile=${outFile}`.cwd(pkgDir)
	const text = await fs.readFile(outFile, 'utf8')
	if (/screenpipe(-mcp)?@latest/.test(text)) {
		throw new Error('MCP bundle still references a registry package')
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
		const staged = await fs.stat(dest).then(() => true, () => false)
		const note = staged ? 'keeping the copy staged earlier' : 'agents will use the REST API instead'
		if (process.env.FREENPIPE_REQUIRE_BUNDLED_RUNTIME === '1' && !staged) {
			throw new Error('no CLI binary: set FREENPIPE_CLI_BINARY or FREENPIPE_BUILD_CLI=1')
		}
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
		const res = await fetch(url)
		if (!res.ok) throw new Error(`${model.filename}: HTTP ${res.status} from ${url}`)
		const body = Buffer.from(await res.arrayBuffer())
		if (!isExpectedModel(body, model)) {
			throw new Error(`${model.filename} from ${url} does not match the pinned size/sha256`)
		}
		await writeAtomically(dest, body)
		console.log(`[bundled-runtime] ${model.filename} downloaded from ${url}`)
	}
}

/**
 * Stage every bundled runtime piece. `srcTauriDir` is apps/.../src-tauri,
 * `platform` is 'windows' | 'macos' | 'linux'.
 */
export async function stageBundledRuntime({ srcTauriDir, platform }) {
	const repoRoot = path.resolve(srcTauriDir, '..', '..', '..')
	const stageRoot = path.join(srcTauriDir, 'bundled')
	// The resource mappings point at these directories, so they must exist
	// even when a step is skipped (an empty directory bundles nothing).
	for (const dir of ['cli', 'mcp', path.join('models', 'pyannote')]) {
		await fs.mkdir(path.join(stageRoot, dir), { recursive: true })
	}
	const required = process.env.FREENPIPE_REQUIRE_BUNDLED_RUNTIME === '1'
	const steps = [
		['MCP server', stageMcp],
		['CLI', stageCli],
		['pyannote models', stagePyannote],
	]
	for (const [label, step] of steps) {
		try {
			await step({ repoRoot, stageRoot, platform })
		} catch (error) {
			if (required) throw new Error(`[bundled-runtime] ${label}: ${error.message}`)
			console.warn(`[bundled-runtime] ${label} not staged: ${error.message}`)
		}
	}
}
