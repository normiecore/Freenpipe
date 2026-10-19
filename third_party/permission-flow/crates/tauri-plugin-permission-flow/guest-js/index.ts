import { invoke, Resource } from '@tauri-apps/api/core'

const PLUGIN_NAME = 'permission-flow'
const RESOURCES_PLUGIN_NAME = 'resources'
const DEFAULT_WATCH_POLL_INTERVAL_MS = 1500

// Best-effort cleanup for handles that become unreachable without an explicit
// `close()`. This complements, but does not replace, deterministic cleanup.
const permissionFlowFinalizer:
  | FinalizationRegistry<number>
  | undefined =
  typeof FinalizationRegistry === 'undefined'
    ? undefined
    : new FinalizationRegistry((rid) => {
        void invoke(`plugin:${RESOURCES_PLUGIN_NAME}|close`, {
          rid,
        }).catch(() => {
          // Finalizers are best-effort cleanup only.
        })
      })

/**
 * Permissions that can be guided through the macOS Settings flow.
 */
export const Permission = {
  Accessibility: 'accessibility',
  InputMonitoring: 'inputMonitoring',
  ScreenRecording: 'screenRecording',
  AppManagement: 'appManagement',
  Bluetooth: 'bluetooth',
  DeveloperTools: 'developerTools',
  FullDiskAccess: 'fullDiskAccess',
  MediaAppleMusic: 'mediaAppleMusic',
} as const

export type Permission = (typeof Permission)[keyof typeof Permission]

/**
 * Options for opening the floating permission guidance flow.
 */
export interface StartFlowOptions {
  permission: Permission
  appPath: string
  useClickSourceFrame?: boolean
}

/**
 * The current host-app status reported by macOS for a permission.
 *
 * This is not the status of the arbitrary `appPath` passed to `startFlow()`.
 * It only describes what the current host app or process can preflight about
 * itself.
 */
export const PermissionAuthorizationState = {
  Granted: 'granted',
  NotGranted: 'notGranted',
  Unknown: 'unknown',
  Checking: 'checking',
} as const

export type PermissionAuthorizationState =
  (typeof PermissionAuthorizationState)[keyof typeof PermissionAuthorizationState]

/**
 * Options for watching host-app permission status over time.
 */
export interface WatchAuthorizationStatusOptions {
  /**
   * Publishes the current status immediately after subscribing.
   *
   * Defaults to `true`, which means the callback still fires when the
   * permission was granted before the app started.
   */
  emitInitial?: boolean
  /**
   * Called whenever a refresh attempt fails.
   */
  onError?: (error: unknown) => void
  /**
   * Background refresh interval in milliseconds.
   *
   * Set to `false` to disable interval refresh and rely only on focus and
   * visibility changes.
   */
  pollIntervalMs?: number | false
}

export type UnwatchAuthorizationStatus = () => void

/**
 * Returns the current host-app status for a permission.
 */
export async function authorizationState(
  permission: Permission
): Promise<PermissionAuthorizationState> {
  return await invoke<PermissionAuthorizationState>(
    `plugin:${PLUGIN_NAME}|authorization_state`,
    { permission }
  )
}

/**
 * Returns a best-effort guess for the host app bundle path in the current
 * launch context.
 */
export async function suggestedHostAppPath(): Promise<string | null> {
  return await invoke<string | null>(
    `plugin:${PLUGIN_NAME}|suggested_host_app_path`
  )
}

/**
 * Watches host-app permission status and republishes only when it changes.
 *
 * By default this immediately emits the current status, refreshes when the
 * window regains focus, refreshes when the page becomes visible again, and
 * keeps a light interval as a safety net.
 */
export function watchAuthorizationStatus(
  permission: Permission,
  onChange: (state: PermissionAuthorizationState) => void,
  options: WatchAuthorizationStatusOptions = {}
): UnwatchAuthorizationStatus {
  const emitInitial = options.emitInitial ?? true
  const pollIntervalMs =
    options.pollIntervalMs === undefined
      ? DEFAULT_WATCH_POLL_INTERVAL_MS
      : options.pollIntervalMs

  let lastState: PermissionAuthorizationState | undefined
  let disposed = false

  const publish = (
    nextState: PermissionAuthorizationState,
    force: boolean = false
  ) => {
    if (disposed) {
      return
    }

    const didChange = nextState !== lastState
    lastState = nextState

    if (force || didChange) {
      onChange(nextState)
    }
  }

  const refresh = async (force: boolean = false) => {
    try {
      publish(await authorizationState(permission), force)
    } catch (error) {
      if (!disposed) {
        options.onError?.(error)
      }
    }
  }

  const handleFocus = () => {
    void refresh()
  }

  const handleVisibilityChange = () => {
    if (typeof document !== 'undefined' && !document.hidden) {
      void refresh()
    }
  }

  if (typeof window !== 'undefined') {
    window.addEventListener('focus', handleFocus)
  }

  if (typeof document !== 'undefined') {
    document.addEventListener('visibilitychange', handleVisibilityChange)
  }

  const intervalId =
    pollIntervalMs === false
      ? undefined
      : globalThis.setInterval(() => {
          if (typeof document === 'undefined' || !document.hidden) {
            void refresh()
          }
        }, pollIntervalMs)

  void refresh(emitInitial)

  return () => {
    disposed = true

    if (typeof window !== 'undefined') {
      window.removeEventListener('focus', handleFocus)
    }

    if (typeof document !== 'undefined') {
      document.removeEventListener('visibilitychange', handleVisibilityChange)
    }

    if (intervalId !== undefined) {
      globalThis.clearInterval(intervalId)
    }
  }
}

/**
 * A Tauri resource handle that owns one native permission-flow controller.
 *
 * Use this when you want to keep a controller alive across multiple button
 * presses without hiding that ownership behind a global singleton.
 */
export class PermissionFlow extends Resource {
  private readonly finalizerToken = {}
  private isClosed = false

  private constructor(rid: number) {
    super(rid)
    permissionFlowFinalizer?.register(this, rid, this.finalizerToken)
  }

  /**
   * Creates a new native controller handle.
   */
  static async create(): Promise<PermissionFlow> {
    const rid = await invoke<number>(`plugin:${PLUGIN_NAME}|create`)
    return new PermissionFlow(rid)
  }

  /**
   * Returns the current host-app status for a permission.
   */
  static async authorizationState(
    permission: Permission
  ): Promise<PermissionAuthorizationState> {
    return await authorizationState(permission)
  }

  /**
   * Returns a best-effort guess for the host app bundle path in the current
   * launch context.
   */
  static async suggestedHostAppPath(): Promise<string | null> {
    return await suggestedHostAppPath()
  }

  /**
   * Watches host-app permission status without requiring the caller to wire
   * their own refresh loop.
   */
  static watchAuthorizationStatus(
    permission: Permission,
    onChange: (state: PermissionAuthorizationState) => void,
    options?: WatchAuthorizationStatusOptions
  ): UnwatchAuthorizationStatus {
    return watchAuthorizationStatus(permission, onChange, options)
  }

  /**
   * Opens the floating guidance flow for a permission.
   */
  async startFlow(options: StartFlowOptions): Promise<void> {
    await invoke(`plugin:${PLUGIN_NAME}|start_flow`, {
      rid: this.rid,
      payload: options,
    })
  }

  /**
   * Closes the active floating guidance flow for this handle, if any.
   */
  async stopCurrentFlow(): Promise<void> {
    await invoke(`plugin:${PLUGIN_NAME}|stop_current_flow`, {
      rid: this.rid,
    })
  }

  /**
   * Deterministically releases the underlying native controller.
   */
  async close(): Promise<void> {
    if (this.isClosed) {
      return
    }

    this.isClosed = true
    permissionFlowFinalizer?.unregister(this.finalizerToken)

    try {
      await super.close()
    } catch (error) {
      this.isClosed = false
      permissionFlowFinalizer?.register(this, this.rid, this.finalizerToken)
      throw error
    }
  }
}

/**
 * Convenience helper for callers who prefer function-style creation.
 */
export async function createPermissionFlow(): Promise<PermissionFlow> {
  return await PermissionFlow.create()
}
