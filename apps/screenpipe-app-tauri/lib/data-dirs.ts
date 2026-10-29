// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

/**
 * Directories the webview reads, resolved by the backend.
 *
 * The backend honours `SCREENPIPE_DATA_DIR` (a dev run's isolated data dir);
 * Tauri's `homeDir()` cannot be redirected. So the webview asks the backend
 * and builds `<home>/.screenpipe` itself only when the backend cannot answer:
 * outside Tauri (tests, a plain browser) or on an older backend without the
 * command. An error the command returns is thrown instead, so a dev run never
 * quietly falls back to the real `~/.screenpipe` store.
 */

import { homeDir, join } from "@tauri-apps/api/path";
import { commands } from "@/lib/utils/tauri";

export type DirCommandResult =
	| { status: "ok"; data: string }
	| { status: "error"; error: string };

/** Whether an invoke error means the backend has no such command. Tauri
 *  rejects an unregistered command with "Command <name> not found". */
export function isMissingCommandError(error: unknown): boolean {
	if (typeof error !== "string") return false;
	const message = error.trim();
	return /^Command \S+ not found$/i.test(message) || /\bCommand not found$/.test(message);
}

/**
 * Resolve a directory through `command`, falling back to `fallback()` only
 * when the command cannot be reached (see the module comment).
 */
export async function resolveDirWith(
	command: () => Promise<DirCommandResult>,
	fallback: () => Promise<string>,
): Promise<string> {
	let res: DirCommandResult;
	try {
		res = await command();
	} catch {
		// The invoke itself failed: no Tauri runtime.
		return fallback();
	}
	if (res.status === "ok") {
		if (res.data) return res.data;
		throw new Error("the backend returned an empty directory");
	}
	if (isMissingCommandError(res.error)) return fallback();
	throw new Error(res.error);
}

/** `<home>/.screenpipe`, the default directory outside Tauri. */
export async function homeScreenpipeDir(): Promise<string> {
	return join(await homeDir(), ".screenpipe");
}

/** Directory of `store.bin`, as the backend resolves it. */
export function resolveStoreDir(): Promise<string> {
	return resolveDirWith(() => commands.getScreenpipeBaseDir(), homeScreenpipeDir);
}

/** Full path of `store.bin`, for code that reads the file itself (the
 *  shortcut overlay) rather than through the settings store. */
export async function resolveStorePath(): Promise<string> {
	return join(await resolveStoreDir(), "store.bin");
}

/**
 * Recording data directory in use (a custom `dataDir` or the default), as
 * the backend resolves it from the settings loaded at startup. `fallback`
 * answers outside Tauri.
 */
export function resolveActiveDataDir(fallback: () => Promise<string>): Promise<string> {
	return resolveDirWith(() => commands.getActiveDataDir(), fallback);
}
