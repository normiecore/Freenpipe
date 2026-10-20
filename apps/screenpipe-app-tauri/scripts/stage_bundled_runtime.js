// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

// Stages the runtime files the Windows installer ships next to the app, under
// src-tauri/bundled/ (mapped as bundle resources in tauri.windows.conf.json):
//
//   bundled/cli/screenpipe(.exe)          -> <install>/screenpipe(.exe)
//   bundled/mcp/screenpipe-mcp.js         -> <install>/mcp/screenpipe-mcp.js
//   bundled/models/pyannote/<model>.onnx  -> <install>/models/pyannote/
//   bundled/models/pyannote/NOTICE.md     (sources and licences of the models)
//   bundled/native/screenpipe_wer_dump_helper.dll -> <install>/ (Windows crash
//                                         dumps, crates/screenpipe-wer-helper)
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
//   FREENPIPE_WER_HELPER_DLL=<path>    copy this prebuilt WER helper DLL (Windows)
//   FREENPIPE_BUILD_WER_HELPER=1       build it: cargo build --release -p screenpipe-wer-helper
//   SCREENPIPE_PYANNOTE_BASE_URL=<url> model mirror (same variable the app reads)
//   FREENPIPE_REQUIRE_BUNDLED_RUNTIME=1  make every step mandatory

import { $ } from 'bun'
import { createHash } from 'crypto'
import fs from 'fs/promises'
import path from 'path'

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

/** File name of the WER helper DLL the app looks for beside its executable. */
export const WER_HELPER_DLL = 'screenpipe_wer_dump_helper.dll'

/**
 * How the WER helper DLL is obtained: `null` when nothing is asked for (dev
 * builds, other platforms), else a prebuilt `source` or a cargo `build`.
 * The build links the C runtime statically: WerFault.exe loads the DLL from
 * System32's search path, where the bundled vcruntime140.dll is not found.
 * It uses its own target directory so those flags do not invalidate the
 * main build cache.
 */
export function werHelperPlan({ repoRoot, platform, env }) {
	if (platform !== 'windows') return null
	if (env.FREENPIPE_WER_HELPER_DLL) return { source: env.FREENPIPE_WER_HELPER_DLL }
	const wanted = env.FREENPIPE_BUILD_WER_HELPER === '1' || env.FREENPIPE_REQUIRE_BUNDLED_RUNTIME === '1'
	if (!wanted) return null
	const targetDir = path.join(env.CARGO_TARGET_DIR || path.join(repoRoot, 'target'), 'wer-helper')
	return {
		build: {
			args: ['build', '--release', '-p', 'screenpipe-wer-helper', '--target-dir', targetDir],
			env: { RUSTFLAGS: [env.RUSTFLAGS, '-C target-feature=+crt-static'].filter(Boolean).join(' ') },
		},
		source: path.join(targetDir, 'release', WER_HELPER_DLL),
	}
}

async function stageWerHelper({ repoRoot, stageRoot, platform }) {
	const dest = path.join(stageRoot, 'native', WER_HELPER_DLL)
	const plan = werHelperPlan({ repoRoot, platform, env: process.env })
	if (!plan) {
		if (platform === 'windows') {
			console.log('[bundled-runtime] WER helper not requested (FREENPIPE_BUILD_WER_HELPER); fail-fast crash dumps stay off')
		}
		return
	}
	if (plan.build) {
		await $`cargo ${plan.build.args}`.cwd(repoRoot).env({ ...process.env, ...plan.build.env })
	}
	await fs.copyFile(plan.source, dest)
	console.log(`[bundled-runtime] WER helper staged from ${plan.source}`)
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

/**
 * Stage every bundled runtime piece. `srcTauriDir` is apps/.../src-tauri,
 * `platform` is 'windows' | 'macos' | 'linux'.
 */
export async function stageBundledRuntime({ srcTauriDir, platform }) {
	const repoRoot = path.resolve(srcTauriDir, '..', '..', '..')
	const stageRoot = path.join(srcTauriDir, 'bundled')
	// The resource mappings point at these directories, so they must exist
	// even when a step is skipped (an empty directory bundles nothing).
	for (const dir of ['cli', 'mcp', 'native', path.join('models', 'pyannote')]) {
		await fs.mkdir(path.join(stageRoot, dir), { recursive: true })
	}
	const required = process.env.FREENPIPE_REQUIRE_BUNDLED_RUNTIME === '1'
	const steps = [
		['MCP server', stageMcp],
		['CLI', stageCli],
		['pyannote models', stagePyannote],
		['WER helper', stageWerHelper],
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
