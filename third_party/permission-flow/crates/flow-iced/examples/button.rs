use iced::widget::{button, column, container, pick_list};
use iced::{Alignment, Fill, Size, Subscription, Task};
use permission_flow_iced::{
    AppPath, Permission, PermissionFlowButton, PermissionFlowButtonMessage, StartFlowOptions,
};

fn main() -> iced::Result {
    iced::application(
        "Permission Flow Iced",
        ExampleHost::update,
        ExampleHost::view,
    )
    .subscription(ExampleHost::subscription)
    .window_size(Size::new(320.0, 140.0))
    .centered()
    .resizable(false)
    .run_with(|| (ExampleHost::new(), Task::none()))
}

#[derive(Debug, Clone)]
enum Message {
    ButtonPressed,
    PermissionSelected(PermissionChoice),
    PermissionButton(PermissionFlowButtonMessage),
}

struct ExampleHost {
    host_app: AppPath,
    button: PermissionFlowButton,
    permission: PermissionChoice,
}

impl ExampleHost {
    fn new() -> Self {
        let host_app = AppPath::suggested_host_app()
            .expect("Expected to find a host app bundle for this permission-flow example");
        let permission = PermissionChoice::Accessibility;
        let button = Self::make_button(host_app.clone(), permission);

        Self {
            host_app,
            button,
            permission,
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ButtonPressed => {
                let _ = self.button.start_flow();
            }
            Message::PermissionSelected(permission) => {
                self.permission = permission;
                self.button = Self::make_button(self.host_app.clone(), permission);
                let _ = self.button.refresh();
            }
            Message::PermissionButton(message) => {
                let _ = self.button.update(&message);
            }
        }

        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        self.button.subscription().map(Message::PermissionButton)
    }

    fn make_button(host_app: AppPath, permission: PermissionChoice) -> PermissionFlowButton {
        PermissionFlowButton::with_options(StartFlowOptions::new(permission.permission(), host_app))
            .expect("The example must be launched on the macOS main thread")
    }

    fn view(&self) -> iced::Element<'_, Message> {
        let button_state = self.button.button_state();
        let content = column![
            pick_list(
                PermissionChoice::ALL,
                Some(self.permission),
                Message::PermissionSelected,
            )
            .width(Fill),
            button(button_state.label())
                .width(Fill)
                .on_press(Message::ButtonPressed),
        ]
        .spacing(14)
        .align_x(Alignment::Center)
        .width(Fill)
        .max_width(240);

        container(content)
            .width(Fill)
            .height(Fill)
            .padding(16)
            .center_x(Fill)
            .center_y(Fill)
            .into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PermissionChoice {
    Accessibility,
    InputMonitoring,
    ScreenRecording,
    AppManagement,
    Bluetooth,
    DeveloperTools,
    FullDiskAccess,
    MediaAppleMusic,
}

impl PermissionChoice {
    const ALL: [Self; 8] = [
        Self::Accessibility,
        Self::InputMonitoring,
        Self::ScreenRecording,
        Self::AppManagement,
        Self::Bluetooth,
        Self::DeveloperTools,
        Self::FullDiskAccess,
        Self::MediaAppleMusic,
    ];

    fn permission(self) -> Permission {
        match self {
            Self::Accessibility => Permission::ACCESSIBILITY,
            Self::InputMonitoring => Permission::INPUT_MONITORING,
            Self::ScreenRecording => Permission::SCREEN_RECORDING,
            Self::AppManagement => Permission::APP_MANAGEMENT,
            Self::Bluetooth => Permission::BLUETOOTH,
            Self::DeveloperTools => Permission::DEVELOPER_TOOLS,
            Self::FullDiskAccess => Permission::FULL_DISK_ACCESS,
            Self::MediaAppleMusic => Permission::MEDIA_APPLE_MUSIC,
        }
    }
}

impl std::fmt::Display for PermissionChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Accessibility => "Accessibility",
            Self::InputMonitoring => "Input Monitoring",
            Self::ScreenRecording => "Screen Recording",
            Self::AppManagement => "App Management",
            Self::Bluetooth => "Bluetooth",
            Self::DeveloperTools => "Developer Tools",
            Self::FullDiskAccess => "Full Disk Access",
            Self::MediaAppleMusic => "Media & Apple Music",
        })
    }
}
