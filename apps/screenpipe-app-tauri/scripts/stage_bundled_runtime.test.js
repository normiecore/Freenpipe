// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

// Run from apps/screenpipe-app-tauri: bun test scripts/stage_bundled_runtime.test.js

import { describe, expect, it } from 'bun:test'
import { mkdtemp, readFile, rm } from 'fs/promises'
import os from 'os'
import path from 'path'
import {
	buildMcpBundle,
	DEFAULT_PYANNOTE_BASE_URL,
	isExpectedModel,
	PYANNOTE_MODELS,
	pyannoteModelUrl,
	sha256Hex,
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

	it('downloads from the Freenpipe release mirror unless overridden', () => {
		const m = PYANNOTE_MODELS[1]
		expect(pyannoteModelUrl(m)).toBe(`${DEFAULT_PYANNOTE_BASE_URL}/${m.asset}`)
		expect(pyannoteModelUrl(m, 'https://mirror.example.test/models/')).toBe(
			`https://mirror.example.test/models/${m.asset}`,
		)
		expect(pyannoteModelUrl(m, 'file:///tmp')).toBe(`${DEFAULT_PYANNOTE_BASE_URL}/${m.asset}`)
		expect(DEFAULT_PYANNOTE_BASE_URL).toStartWith('https://github.com/normiecore/Freenpipe/')
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
