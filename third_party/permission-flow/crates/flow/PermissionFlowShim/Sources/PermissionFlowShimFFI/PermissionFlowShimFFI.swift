@preconcurrency import AppKit
@preconcurrency import ApplicationServices
import Carbon
import CoreBluetooth
import Foundation
@preconcurrency import PermissionFlow
import StoreKit
#if canImport(MusicKit)
import MusicKit
#endif

// Permission:
//  - 1 accessibility
//  - 2 input-monitoring
//  - 3 screen-recording
//  - 4 app-management
//  - 5 bluetooth
//  - 6 developer-tools
//  - 7 full-disk-access
//  - 8 media-apple-music
//
// RETURN:
//  - 0 OK
//  - 1 invalid permission pane
//  - 2 null controller
//  - 3 not on main thread

private enum PermissionFlowShimStatus: Int8 {
    case ok = 0
    case invalidPermissionPane = 1
    case nullController = 2
    case notMainThread = 3
}

private enum PermissionFlowShimAuthorizationState: Int8 {
    case granted = 0
    case notGranted = 1
    case unknown = 2
    case checking = 3
}

private enum PermissionFlowShimPermission: Int8 {
    case accessibility = 1
    case inputMonitoring = 2
    case screenRecording = 3
    case appManagement = 4
    case bluetooth = 5
    case developerTools = 6
    case fullDiskAccess = 7
    case mediaAppleMusic = 8

    var pane: PermissionFlowPane {
        switch self {
        case .accessibility:
            return .accessibility
        case .inputMonitoring:
            return .inputMonitoring
        case .screenRecording:
            return .screenRecording
        case .appManagement:
            return .appManagement
        case .bluetooth:
            return .bluetooth
        case .developerTools:
            return .developerTools
        case .fullDiskAccess:
            return .fullDiskAccess
        case .mediaAppleMusic:
            return .mediaAppleMusic
        }
    }
}

private struct ControllerPointerOutBox: @unchecked Sendable {
    let value: UnsafeMutablePointer<UnsafeMutableRawPointer?>?
}

@inline(__always)
private func requireMainThread() -> PermissionFlowShimStatus? {
    Thread.isMainThread ? nil : .notMainThread
}

@_cdecl("permission_flow_controller_new")
public func permission_flow_controller_new(
    _ controllerPointerOut: UnsafeMutablePointer<UnsafeMutableRawPointer?>?
) -> Int8 {
    let controllerPointerOutBox = ControllerPointerOutBox(value: controllerPointerOut)

    guard let controllerPointerOut else {
        return PermissionFlowShimStatus.nullController.rawValue
    }
    guard let status = requireMainThread() else {
        return MainActor.assumeIsolated {
            let controller = PermissionFlow.makeController(
                configuration: .init(promptForAccessibilityTrust: false)
            )
            controllerPointerOutBox.value?.pointee = UnsafeMutableRawPointer(Unmanaged.passRetained(controller).toOpaque())
            return PermissionFlowShimStatus.ok.rawValue
        }
    }

    controllerPointerOut.pointee = nil
    return status.rawValue
}

@_cdecl("permission_flow_controller_free")
@MainActor
public func permission_flow_controller_free(
    _ controllerPointer: UnsafeMutableRawPointer?
) -> Int8 {
    guard let controllerPointer else {
        return PermissionFlowShimStatus.nullController.rawValue
    }

    let controller = Unmanaged<PermissionFlowController>
        .fromOpaque(controllerPointer)
        .takeRetainedValue()
    controller.closePanel()
    return PermissionFlowShimStatus.ok.rawValue
}

@_cdecl("permission_flow_controller_start_flow")
@MainActor
public func permission_flow_controller_start_flow(
    _ controllerPointer: UnsafeMutableRawPointer?,
    _ permissionEnum: Int8,
    _ appPathPointer: UnsafePointer<CChar>?,
    _ useClickSourceFrame: Int8
) -> Int8 {
    startFlowImpl(
        controllerPointer,
        permissionEnum,
        appPathPointer,
        useClickSourceFrame: useClickSourceFrame != 0
    )
}

@_cdecl("permission_flow_controller_close_panel")
@MainActor
public func permission_flow_controller_close_panel(
    _ controllerPointer: UnsafeMutableRawPointer?
) -> Int8 {
    guard let controllerPointer else {
        return PermissionFlowShimStatus.nullController.rawValue
    }

    let controller = Unmanaged<PermissionFlowController>
        .fromOpaque(controllerPointer)
        .takeUnretainedValue()
    controller.closePanel()
    return PermissionFlowShimStatus.ok.rawValue
}

@_cdecl("permission_flow_authorization_state")
public func permission_flow_authorization_state(
    _ permissionEnum: Int8,
    _ authorizationStateOut: UnsafeMutablePointer<Int8>?
) -> Int8 {
    guard let authorizationStateOut else {
        return PermissionFlowShimStatus.nullController.rawValue
    }
    guard let permission = PermissionFlowShimPermission(rawValue: permissionEnum) else {
        return PermissionFlowShimStatus.invalidPermissionPane.rawValue
    }

    authorizationStateOut.pointee = authorizationState(for: permission).rawValue
    return PermissionFlowShimStatus.ok.rawValue
}

@MainActor
@inline(__always)
private func startFlowImpl(
    _ controllerPointer: UnsafeMutableRawPointer?,
    _ permissionEnum: Int8,
    _ appPathPointer: UnsafePointer<CChar>?,
    useClickSourceFrame: Bool
) -> Int8 {
    guard let controllerPointer else {
        return PermissionFlowShimStatus.nullController.rawValue
    }

    guard let permission = PermissionFlowShimPermission(rawValue: permissionEnum) else {
        return PermissionFlowShimStatus.invalidPermissionPane.rawValue
    }

    let suggestedAppURLs = suggestedAppURLs(from: appPathPointer)

    let controller = Unmanaged<PermissionFlowController>
        .fromOpaque(controllerPointer)
        .takeUnretainedValue()

    controller.authorize(
        pane: permission.pane,
        suggestedAppURLs: suggestedAppURLs,
        sourceFrameInScreen: useClickSourceFrame ? clickSourceFrameInScreen() : nil
    )

    return PermissionFlowShimStatus.ok.rawValue
}

private func suggestedAppURLs(from appPathPointer: UnsafePointer<CChar>?) -> [URL] {
    guard let appPathPointer else {
        return []
    }

    let appPath = String(cString: appPathPointer)
    guard appPath.isEmpty == false else {
        return []
    }

    // Normalize the bundle path before handing it to PermissionFlow.
    return [URL(fileURLWithPath: appPath).standardizedFileURL]
}

private func authorizationState(
    for permission: PermissionFlowShimPermission
) -> PermissionFlowShimAuthorizationState {
    switch permission {
    case .accessibility:
        return AXIsProcessTrusted() ? .granted : .notGranted
    case .inputMonitoring:
        return CGPreflightListenEventAccess() ? .granted : .notGranted
    case .screenRecording:
        return CGPreflightScreenCaptureAccess() ? .granted : .notGranted
    case .appManagement, .developerTools:
        return .unknown
    case .bluetooth:
        switch CBManager.authorization {
        case .allowedAlways:
            return .granted
        case .denied, .restricted, .notDetermined:
            return .notGranted
        @unknown default:
            return .unknown
        }
    case .fullDiskAccess:
        return fullDiskAccessAuthorizationState()
    case .mediaAppleMusic:
        #if canImport(MusicKit)
        switch MusicAuthorization.currentStatus {
        case .authorized:
            return .granted
        case .denied, .restricted, .notDetermined:
            return .notGranted
        @unknown default:
            return .unknown
        }
        #else
        switch SKCloudServiceController.authorizationStatus() {
        case .authorized:
            return .granted
        case .denied, .restricted, .notDetermined:
            return .notGranted
        @unknown default:
            return .unknown
        }
        #endif
    }
}

private func fullDiskAccessAuthorizationState() -> PermissionFlowShimAuthorizationState {
    let rootHome = "/var/root"
    if FileManager.default.isReadableFile(atPath: rootHome) {
        return .granted
    }

    let systemConfigPaths = [
        "/private/etc/sudoers",
        "/Library/Application Support/com.apple.TCC/TCC.db",
        "/private/var/db/SystemPolicy"
    ]

    for path in systemConfigPaths where FileManager.default.isReadableFile(atPath: path) {
        return .granted
    }

    do {
        let usernames = try FileManager.default.contentsOfDirectory(atPath: "/Users")
        let currentUser = NSUserName()

        for username in usernames {
            if username == currentUser || username == "Shared" || username.hasPrefix(".") {
                continue
            }

            let otherUserHome = "/Users/\(username)"
            if FileManager.default.isReadableFile(atPath: otherUserHome) {
                do {
                    _ = try FileManager.default.contentsOfDirectory(atPath: otherUserHome)
                    return .granted
                } catch {
                    continue
                }
            }
        }
    } catch {
        // If /Users cannot be listed, fall through to the remaining heuristics.
    }

    let protectedUserPaths = [
        "/Users/\(NSUserName())/Library/Mail",
        "/Users/\(NSUserName())/Library/Safari/Databases",
        "/Users/\(NSUserName())/Library/Messages",
        "/Users/\(NSUserName())/Library/Application Support/com.apple.sharedfilelist"
    ]

    for path in protectedUserPaths {
        if FileManager.default.fileExists(atPath: path) {
            do {
                _ = try FileManager.default.contentsOfDirectory(atPath: path)
                return .granted
            } catch {
                continue
            }
        }
    }

    return .notGranted
}

@MainActor
private func clickSourceFrameInScreen() -> CGRect {
    let mouse = NSEvent.mouseLocation
    return CGRect(x: mouse.x - 16, y: mouse.y - 16, width: 32, height: 32)
}
