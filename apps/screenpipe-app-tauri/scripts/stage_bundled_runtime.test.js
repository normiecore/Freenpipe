// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

// Run from apps/screenpipe-app-tauri: bun test scripts/stage_bundled_runtime.test.js

import { describe, expect, it } from 'bun:test'
import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'fs/promises'
import os from 'os'
import path from 'path'
import { deflateRawSync } from 'zlib'
import {
	assertCleanMcpBundle,
	assertDllForArch,
	buildMcpBundle,
	DEFAULT_PYANNOTE_BASE_URL,
	DIRECTML_PACKAGE,
	isExpectedModel,
	MODEL_NOTICE,
	ORT_DIRECTML_PACKAGE,
	PE_MACHINE,
	peMachine,
	pickVulkanRuntimeFiles,
	PYANNOTE_MODELS,
	pyannoteModelUrl,
	readZipEntries,
	sha256Hex,
	SILERO_MODEL,
	stageDirectMl,
	stageVulkanLoader,
	VULKAN_LOADER,
	WER_HELPER_DLL,
	werHelperPlan,
	windowsArch,
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

	it('downloads from the Handvel LFS mirror unless overridden', () => {
		const m = PYANNOTE_MODELS[1]
		expect(pyannoteModelUrl(m)).toBe(`${DEFAULT_PYANNOTE_BASE_URL}/${m.asset}`)
		expect(pyannoteModelUrl(m, 'https://mirror.example.test/models/')).toBe(
			`https://mirror.example.test/models/${m.asset}`,
		)
		expect(pyannoteModelUrl(m, 'file:///tmp')).toBe(`${DEFAULT_PYANNOTE_BASE_URL}/${m.asset}`)
		expect(DEFAULT_PYANNOTE_BASE_URL).toMatch(
			/^https:\/\/media\.githubusercontent\.com\/media\/normiecore\/Handvel\/[0-9a-f]{40}\/crates\/screenpipe-audio\/models\/pyannote$/,
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

describe('silero VAD model', () => {
	it('matches the pin in the Rust VAD loader', async () => {
		const rust = await readFile(path.join(repoRoot, 'crates/screenpipe-audio/src/vad/silero.rs'), 'utf8')
		expect(rust).toContain(`"${SILERO_MODEL.filename}"`)
		expect(rust).toContain(`"${SILERO_MODEL.url}"`)
		expect(rust).toContain(`sha256: "${SILERO_MODEL.sha256}"`)
		expect(rust.replaceAll('_', '')).toContain(`size: ${SILERO_MODEL.size}`)
	})

	it('is pinned to a commit of the original publisher', () => {
		expect(SILERO_MODEL.url).toMatch(
			/^https:\/\/raw\.githubusercontent\.com\/snakers4\/silero-vad\/[0-9a-f]{40}\/src\/silero_vad\/data\/silero_vad\.onnx$/,
		)
	})

	it('ships an attribution notice naming the file, its source commit and hash', async () => {
		const notice = await readFile(path.join(repoRoot, 'crates/screenpipe-audio/models/silero', MODEL_NOTICE), 'utf8')
		const commit = SILERO_MODEL.url.split('/')[5]
		expect(notice).toContain(SILERO_MODEL.filename)
		expect(notice).toContain(SILERO_MODEL.sha256)
		expect(notice).toContain(commit)
		expect(notice).toContain('MIT')
	})
})

/** Minimal zip writer (stored or deflated entries) for the reader tests. */
function makeZip(entries) {
	const parts = []
	const central = []
	let offset = 0
	for (const e of entries) {
		const name = Buffer.from(e.name)
		const body = e.method === 8 ? deflateRawSync(e.data) : e.data
		const local = Buffer.alloc(30)
		local.writeUInt32LE(0x04034b50, 0)
		local.writeUInt16LE(20, 4)
		local.writeUInt16LE(e.method, 8)
		local.writeUInt32LE(body.length, 18)
		local.writeUInt32LE(e.data.length, 22)
		local.writeUInt16LE(name.length, 26)
		const dir = Buffer.alloc(46)
		dir.writeUInt32LE(0x02014b50, 0)
		dir.writeUInt16LE(e.method, 10)
		dir.writeUInt32LE(body.length, 20)
		dir.writeUInt32LE(e.data.length, 24)
		dir.writeUInt16LE(name.length, 28)
		dir.writeUInt32LE(offset, 42)
		parts.push(local, name, body)
		central.push(dir, name)
		offset += 30 + name.length + body.length
	}
	const cd = Buffer.concat(central)
	const end = Buffer.alloc(22)
	end.writeUInt32LE(0x06054b50, 0)
	end.writeUInt16LE(entries.length, 8)
	end.writeUInt16LE(entries.length, 10)
	end.writeUInt32LE(cd.length, 12)
	end.writeUInt32LE(offset, 16)
	return Buffer.concat([...parts, cd, end])
}

/** Smallest buffer peMachine() accepts as a PE image for `machine`. */
function fakeDll(machine) {
	const b = Buffer.alloc(0x100)
	b.writeUInt16LE(0x5a4d, 0)
	b.writeUInt32LE(0x80, 0x3c)
	b.writeUInt32LE(0x00004550, 0x80)
	b.writeUInt16LE(machine, 0x84)
	return b
}

describe('Windows GPU runtime', () => {
	it('reads stored and deflated zip entries and reports missing ones', () => {
		const big = Buffer.from('x'.repeat(5000))
		const zip = makeZip([
			{ name: 'LICENSE', data: Buffer.from('MIT'), method: 0 },
			{ name: 'runtimes/win-x64/native/onnxruntime.dll', data: big, method: 8 },
		])
		const got = readZipEntries(zip, ['LICENSE', 'runtimes/win-x64/native/onnxruntime.dll'])
		expect(got.get('LICENSE').toString()).toBe('MIT')
		expect(got.get('runtimes/win-x64/native/onnxruntime.dll').equals(big)).toBe(true)
		expect(() => readZipEntries(zip, ['DirectML.dll'])).toThrow(/not in the archive/)
		expect(() => readZipEntries(Buffer.from('not a zip at all, just text....'), ['x'])).toThrow(/not a zip/)
		const odd = makeZip([{ name: 'a', data: Buffer.from('a'), method: 0 }])
		odd.writeUInt16LE(14, odd.length - 22 - 47 + 10) // central entry method -> LZMA
		expect(() => readZipEntries(odd, ['a'])).toThrow(/compression method 14/)
	})

	it('checks that a DLL is built for the target architecture', () => {
		expect(peMachine(fakeDll(PE_MACHINE.x64))).toBe(0x8664)
		expect(peMachine(Buffer.from('MZ but too short'))).toBe(null)
		expect(() => assertDllForArch(fakeDll(PE_MACHINE.x64), 'x64', 'a.dll')).not.toThrow()
		expect(() => assertDllForArch(fakeDll(PE_MACHINE.arm64), 'x64', 'a.dll')).toThrow(/not x64/)
		expect(() => assertDllForArch(fakeDll(0x14c), 'arm64', 'a.dll')).toThrow(/not arm64/) // x86
		expect(() => assertDllForArch(Buffer.alloc(0x100), 'x64', 'a.dll')).toThrow(/not a Windows PE/)
	})

	it('derives the architecture from the release target', () => {
		expect(windowsArch('aarch64-pc-windows-msvc', 'x64')).toBe('arm64')
		expect(windowsArch('x86_64-pc-windows-msvc', 'arm64')).toBe('x64')
		expect(windowsArch(undefined, 'arm64')).toBe('arm64')
		expect(windowsArch('', 'x64')).toBe('x64')
	})

	it('pins every runtime package by version, size and hash on nuget.org', () => {
		for (const pkg of [ORT_DIRECTML_PACKAGE, DIRECTML_PACKAGE]) {
			expect(pkg.url).toMatch(/^https:\/\/api\.nuget\.org\/v3-flatcontainer\/[a-z.]+\/\d+\.\d+\.\d+\/[a-z.]+\.\d+\.\d+\.\d+\.nupkg$/)
			expect(pkg.sha256).toMatch(/^[0-9a-f]{64}$/)
			expect(pkg.size).toBeGreaterThan(1_000_000)
			for (const arch of ['x64', 'arm64']) {
				const files = pkg.files(arch)
				expect(files.some((f) => f.dll)).toBe(true)
				// Every DLL travels with a licence text.
				expect(files.some((f) => /licen[cs]e/i.test(f.dest))).toBe(true)
			}
		}
		expect(ORT_DIRECTML_PACKAGE.files('x64').map((f) => f.dest)).toContain('onnxruntime_providers_shared.dll')
	})

	it('finds the Vulkan loader for the architecture and refuses one without a licence', () => {
		const lunarLayout = [
			'VulkanRT-1.4.313.0-Components/LICENSE.txt',
			'VulkanRT-1.4.313.0-Components/x86/vulkan-1.dll',
			'VulkanRT-1.4.313.0-Components/x64/vulkan-1.dll',
			'VulkanRT-1.4.313.0-Components/ARM64/vulkan-1.dll',
			'VulkanRT-1.4.313.0-Components/x64/vulkaninfo.exe',
		]
		expect(pickVulkanRuntimeFiles(lunarLayout, 'x64')).toEqual({
			dll: 'VulkanRT-1.4.313.0-Components/x64/vulkan-1.dll',
			licence: 'VulkanRT-1.4.313.0-Components/LICENSE.txt',
		})
		expect(pickVulkanRuntimeFiles(lunarLayout, 'arm64').dll).toBe('VulkanRT-1.4.313.0-Components/ARM64/vulkan-1.dll')
		expect(pickVulkanRuntimeFiles(['vulkan-1.dll', 'docs/LICENSE.md', 'LICENSE'], 'x64')).toEqual({
			dll: 'vulkan-1.dll',
			licence: 'LICENSE',
		})
		expect(() => pickVulkanRuntimeFiles(['x86/vulkan-1.dll', 'LICENSE.txt'], 'x64')).toThrow(/for x64/)
		expect(() => pickVulkanRuntimeFiles(['x64/vulkan-1.dll'], 'x64')).toThrow(/licence/)
		expect(() => pickVulkanRuntimeFiles(['README.txt'], 'x64')).toThrow(/no vulkan-1.dll/)
	})

	it('stages the Vulkan loader with its licence under the install names', async () => {
		const dir = await mkdtemp(path.join(os.tmpdir(), 'fp-vk-'))
		const prev = process.env.FREENPIPE_VULKAN_RUNTIME_DIR
		try {
			const source = path.join(dir, 'source')
			await mkdir(path.join(source, 'rt', 'x64'), { recursive: true })
			await writeFile(path.join(source, 'rt', 'x64', 'vulkan-1.dll'), fakeDll(PE_MACHINE.x64))
			await writeFile(path.join(source, 'rt', 'LICENSE.txt'), 'Apache License 2.0')
			const stageRoot = path.join(dir, 'bundled')
			await mkdir(path.join(stageRoot, 'runtime'), { recursive: true })
			process.env.FREENPIPE_VULKAN_RUNTIME_DIR = source
			await stageVulkanLoader({ stageRoot, platform: 'windows', arch: 'x64' })
			const runtime = path.join(stageRoot, 'runtime')
			expect((await readFile(path.join(runtime, VULKAN_LOADER.dll))).equals(fakeDll(PE_MACHINE.x64))).toBe(true)
			expect(await readFile(path.join(runtime, VULKAN_LOADER.licence), 'utf8')).toBe('Apache License 2.0')
			// An arm64 build must not pick up the x64 loader.
			await expect(stageVulkanLoader({ stageRoot, platform: 'windows', arch: 'arm64' })).rejects.toThrow(/arm64/)
			// Other platforms stage nothing.
			await stageVulkanLoader({ stageRoot: path.join(dir, 'other'), platform: 'linux', arch: 'x64' })
			expect(await stat(path.join(dir, 'other')).catch(() => null)).toBe(null)
		} finally {
			if (prev === undefined) delete process.env.FREENPIPE_VULKAN_RUNTIME_DIR
			else process.env.FREENPIPE_VULKAN_RUNTIME_DIR = prev
			await rm(dir, { recursive: true, force: true })
		}
	})

	it('removes a stale DirectML.dll unless bundling it was asked for', async () => {
		const dir = await mkdtemp(path.join(os.tmpdir(), 'fp-dml-'))
		const prev = process.env.FREENPIPE_BUNDLE_DIRECTML
		try {
			const runtime = path.join(dir, 'runtime')
			await mkdir(runtime, { recursive: true })
			await writeFile(path.join(runtime, 'DirectML.dll'), fakeDll(PE_MACHINE.x64))
			await writeFile(path.join(runtime, 'DirectML-License.txt'), 'terms')
			await writeFile(path.join(runtime, 'onnxruntime.dll'), fakeDll(PE_MACHINE.x64))
			delete process.env.FREENPIPE_BUNDLE_DIRECTML
			await stageDirectMl({ stageRoot: dir, platform: 'windows', arch: 'x64' })
			expect(await stat(path.join(runtime, 'DirectML.dll')).catch(() => null)).toBe(null)
			expect(await stat(path.join(runtime, 'DirectML-License.txt')).catch(() => null)).toBe(null)
			expect(await stat(path.join(runtime, 'onnxruntime.dll'))).toBeTruthy()
		} finally {
			if (prev === undefined) delete process.env.FREENPIPE_BUNDLE_DIRECTML
			else process.env.FREENPIPE_BUNDLE_DIRECTML = prev
			await rm(dir, { recursive: true, force: true })
		}
	})

	it('maps the staged runtime into the install root and drops the old ONNX Runtime glob', async () => {
		const conf = JSON.parse(await readFile(path.join(repoRoot, 'apps/screenpipe-app-tauri/src-tauri/tauri.windows.conf.json'), 'utf8'))
		const resources = conf.bundle.resources
		expect(resources['bundled\\runtime']).toBe('./')
		expect(resources['bundled\\vad']).toBe('./vad/')
		expect(resources['bundled\\models']).toBe('./models/')
		// onnxruntime.dll comes from bundled/runtime only; a second mapping to
		// the same install path would make the shipped build ambiguous.
		expect(Object.keys(resources).filter((k) => k.toLowerCase().includes('onnxruntime'))).toEqual([])
	})
})
