// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

// Run from apps/screenpipe-app-tauri: bun test scripts/stage_bundled_runtime.test.js

import { describe, expect, it } from 'bun:test'
import { mkdtemp, readFile, rm } from 'fs/promises'
import os from 'os'
import path from 'path'
import {
	assertCleanMcpBundle,
	buildMcpBundle,
	DEFAULT_PYANNOTE_BASE_URL,
	isExpectedModel,
	MODEL_NOTICE,
	PYANNOTE_MODELS,
	pyannoteModelUrl,
	sha256Hex,
	WER_HELPER_DLL,
	werHelperPlan,
} from './stage_bundled_runtime.js'

const repoRoot = path.resolve(import.meta.dir, '..', '..', '..')

describe('pyannote model table', () => {
	it('matches the pins in the Rust model loader', async () => {
		const rust = await readFile(
			path.join(repoRoot, 'crates/screenpipe-audio/src/speaker/models.rs'),
			'utf8',
		)
		expect(rust).toContain(`"${DEFAULT_PYANNOTE_BASE_URL}"`)
		for (const m of PYANNOTE_MODELS) {
			expect(rust).toContain(`filename: "${m.filename}"`)
			expect(rust).toContain(`asset_name: "${m.asset}"`)
			expect(rust).toContain(`sha256: "${m.sha256}"`)
			expect(rust.replaceAll('_', '')).toContain(`size: ${m.size}`)
		}
	})

	it('matches the Git LFS pointers committed in the repository', async () => {
		for (const m of PYANNOTE_MODELS) {
			const file = path.join(repoRoot, 'crates/screenpipe-audio/models/pyannote', m.filename)
			const text = await readFile(file, 'latin1')
			if (!text.startsWith('version https://git-lfs')) continue // real content checked out
			expect(text).toContain(`oid sha256:${m.sha256}`)
			expect(text).toContain(`size ${m.size}`)
		}
	})

	it('downloads from the Freenpipe LFS mirror unless overridden', () => {
		const m = PYANNOTE_MODELS[1]
		expect(pyannoteModelUrl(m)).toBe(`${DEFAULT_PYANNOTE_BASE_URL}/${m.asset}`)
		expect(pyannoteModelUrl(m, 'https://mirror.example.test/models/')).toBe(
			`https://mirror.example.test/models/${m.asset}`,
		)
		expect(pyannoteModelUrl(m, 'file:///tmp')).toBe(`${DEFAULT_PYANNOTE_BASE_URL}/${m.asset}`)
		expect(DEFAULT_PYANNOTE_BASE_URL).toMatch(
			/^https:\/\/media\.githubusercontent\.com\/media\/normiecore\/Freenpipe\/[0-9a-f]{40}\/crates\/screenpipe-audio\/models\/pyannote$/,
		)
	})

	it('names each file on the mirror after the repository copy', () => {
		for (const m of PYANNOTE_MODELS) {
			expect(decodeURIComponent(m.asset)).toBe(m.filename)
			expect(encodeURIComponent(m.filename)).toBe(m.asset)
		}
	})

	it('ships an attribution notice naming every model and its hash', async () => {
		const notice = await readFile(
			path.join(repoRoot, 'crates/screenpipe-audio/models/pyannote', MODEL_NOTICE),
			'utf8',
		)
		for (const m of PYANNOTE_MODELS) {
			expect(notice).toContain(m.filename)
			expect(notice).toContain(m.sha256)
		}
		expect(notice).toContain('CC BY 4.0')
	})

	it('accepts only the exact pinned bytes', () => {
		const body = Buffer.from('abc')
		const model = { size: 3, sha256: sha256Hex(body) }
		expect(isExpectedModel(body, model)).toBe(true)
		expect(isExpectedModel(Buffer.from('abd'), model)).toBe(false)
		expect(isExpectedModel(Buffer.from('ab'), model)).toBe(false)
	})
})

describe('bundled MCP server', () => {
	it('rejects bundles that reach a registry package or a vendor host', () => {
		expect(() => assertCleanMcpBundle('const API = "http://localhost:3030"')).not.toThrow()
		// Built from parts so the repository-wide grep for registry
		// references (W0-09 acceptance) does not match this test.
		const registryRef = ['screenpipe-mcp', 'latest'].join('@')
		expect(() => assertCleanMcpBundle(`spawn("npx", ["${registryRef}"])`)).toThrow(/registry/)
		expect(() => assertCleanMcpBundle('fetch("https://screenpi.pe/api/x")')).toThrow(/vendor/)
		expect(() => assertCleanMcpBundle('const u = "https://API.SCREENPIPE.COM/v1"')).toThrow(/vendor/)
	})

	it(
		'builds into one file that answers tools/list over stdio',
		async () => {
			const dir = await mkdtemp(path.join(os.tmpdir(), 'fp-mcp-'))
			try {
				const entry = await buildMcpBundle(repoRoot, path.join(dir, 'mcp', 'screenpipe-mcp.js'))
				const proc = Bun.spawn([process.execPath, entry, '--port', '3035'], {
					stdin: 'pipe',
					stdout: 'pipe',
					stderr: 'ignore',
					env: { ...process.env, SCREENPIPE_LOCAL_API_KEY: 'sp-test0000' },
				})
				const send = (msg) => proc.stdin.write(`${JSON.stringify(msg)}\n`)
				send({
					jsonrpc: '2.0',
					id: 1,
					method: 'initialize',
					params: {
						protocolVersion: '2025-03-26',
						capabilities: {},
						clientInfo: { name: 'stage-test', version: '0' },
					},
				})
				send({ jsonrpc: '2.0', method: 'notifications/initialized' })
				send({ jsonrpc: '2.0', id: 2, method: 'tools/list' })
				proc.stdin.flush()

				const decoder = new TextDecoder()
				let buffered = ''
				let tools = null
				for await (const chunk of proc.stdout) {
					buffered += decoder.decode(chunk)
					const line = buffered.split('\n').find((l) => l.includes('"id":2'))
					if (line) {
						tools = JSON.parse(line).result.tools
						break
					}
				}
				proc.kill()
				expect(Array.isArray(tools)).toBe(true)
				expect(tools.map((t) => t.name)).toContain('search-content')
			} finally {
				await rm(dir, { recursive: true, force: true })
			}
		},
		{ timeout: 180_000 },
	)
})

describe('WER helper DLL', () => {
	const root = path.join(os.tmpdir(), 'fp-repo')

	it('is only staged for Windows builds that ask for it', () => {
		expect(werHelperPlan({ repoRoot: root, platform: 'linux', env: { FREENPIPE_BUILD_WER_HELPER: '1' } })).toBeNull()
		expect(werHelperPlan({ repoRoot: root, platform: 'windows', env: {} })).toBeNull()
		expect(werHelperPlan({ repoRoot: root, platform: 'windows', env: { FREENPIPE_WER_HELPER_DLL: 'C:/x.dll' } })).toEqual({
			source: 'C:/x.dll',
		})
	})

	it('builds with a static C runtime in its own target directory', () => {
		const plan = werHelperPlan({
			repoRoot: root,
			platform: 'windows',
			env: { FREENPIPE_BUILD_WER_HELPER: '1', RUSTFLAGS: '-C debuginfo=0' },
		})
		const targetDir = path.join(root, 'target', 'wer-helper')
		expect(plan.build.args).toEqual(['build', '--release', '-p', 'screenpipe-wer-helper', '--target-dir', targetDir])
		expect(plan.build.env.RUSTFLAGS).toBe('-C debuginfo=0 -C target-feature=+crt-static')
		expect(plan.source).toBe(path.join(targetDir, 'release', WER_HELPER_DLL))
		// Release builds that require every bundled piece build it too.
		const required = werHelperPlan({
			repoRoot: root,
			platform: 'windows',
			env: { FREENPIPE_REQUIRE_BUNDLED_RUNTIME: '1', CARGO_TARGET_DIR: '/cache/target' },
		})
		expect(required.source).toBe(path.join('/cache/target', 'wer-helper', 'release', WER_HELPER_DLL))
	})

	it('builds for the release matrix triple when one is named', () => {
		const targetDir = path.join(root, 'target', 'wer-helper')
		const triple = 'x86_64-pc-windows-msvc'
		const plan = werHelperPlan({
			repoRoot: root,
			platform: 'windows',
			env: { FREENPIPE_REQUIRE_BUNDLED_RUNTIME: '1', SCREENPIPE_RELEASE_TARGET: triple },
		})
		expect(plan.build.args).toEqual([
			'build',
			'--release',
			'-p',
			'screenpipe-wer-helper',
			'--target-dir',
			targetDir,
			'--target',
			triple,
		])
		expect(plan.source).toBe(path.join(targetDir, triple, 'release', WER_HELPER_DLL))
		// An empty value behaves like an unset one.
		const unset = werHelperPlan({
			repoRoot: root,
			platform: 'windows',
			env: { FREENPIPE_BUILD_WER_HELPER: '1', SCREENPIPE_RELEASE_TARGET: '' },
		})
		expect(unset.build.args).not.toContain('--target')
		expect(unset.source).toBe(path.join(targetDir, 'release', WER_HELPER_DLL))
	})

	it('matches the crate, the app and the Windows bundle mapping', async () => {
		const cargo = await readFile(path.join(repoRoot, 'crates/screenpipe-wer-helper/Cargo.toml'), 'utf8')
		expect(cargo).toContain(`name = "${WER_HELPER_DLL.replace(/\.dll$/, '')}"`)
		expect(cargo).toContain('name = "screenpipe-wer-helper"')
		const wer = await readFile(path.join(repoRoot, 'apps/screenpipe-app-tauri/src-tauri/src/wer.rs'), 'utf8')
		expect(wer).toContain(`pub const HELPER_DLL_NAME: &str = "${WER_HELPER_DLL}";`)
		const conf = JSON.parse(
			await readFile(path.join(repoRoot, 'apps/screenpipe-app-tauri/src-tauri/tauri.windows.conf.json'), 'utf8'),
		)
		expect(conf.bundle.resources['bundled\\native']).toBe('./')
	})
})
