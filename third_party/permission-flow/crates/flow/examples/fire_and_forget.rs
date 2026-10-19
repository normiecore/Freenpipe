use iced::widget::{button, column, container, text};
use iced::{Element, Fill, Size, Task};
use permission_flow::{AppPath, Permission, PermissionFlowController, StartFlowOptions};

pub fn run_button_host<F, E>(
    title: &'static str,
    button_label: &'static str,
    mut on_press: F,
) -> iced::Result
where
    F: FnMut() -> Result<(), E> + 'static,
    E: std::fmt::Display,
{
    let action = Box::new(move || on_press().map_err(|error| error.to_string()));

    iced::application(title, ExampleHost::update, ExampleHost::view)
        .window_size(Size::new(320.0, 180.0))
        .centered()
        .resizable(false)
        .run_with(move || (ExampleHost::new(button_label, action), Task::none()))
}

type ExampleAction = dyn FnMut() -> Result<(), String>;

#[derive(Debug, Clone, Copy)]
enum Message {
    ButtonPressed,
}

struct ExampleHost {
    button_label: &'static str,
    action: Box<ExampleAction>,
    status: String,
}

impl ExampleHost {
    fn new(button_label: &'static str, action: Box<ExampleAction>) -> Self {
        Self {
            button_label,
            action,
            status: "Click the button to start the permission flow.".to_owned(),
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ButtonPressed => {
                self.status = "Opening permission flow...".to_owned();
                match (self.action)() {
                    Ok(()) => {
                        self.status =
                            "Permission flow started. Interact with the floating panel.".to_owned();
                    }
                    Err(error) => {
                        self.status = format!("Error: {error}");
                    }
                }
            }
        }

        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let card = column![
            text("Permission Flow").size(22),
            text(&self.status).size(14),
            button(self.button_label).on_press(Message::ButtonPressed),
        ]
        .spacing(12)
        .width(Fill);

        container(card)
            .width(Fill)
            .height(Fill)
            .padding(20)
            .center_x(Fill)
            .center_y(Fill)
            .into()
    }
}

fn main() -> iced::Result {
    let flow = PermissionFlowController::new().unwrap();
    let host_app = AppPath::suggested_host_app()
        .expect("Expected to find a host app bundle for this permission-flow example");

    run_button_host("Fire And Forget", "Accessibility", move || {
        flow.start_flow(StartFlowOptions::new(
            Permission::ACCESSIBILITY,
            host_app.clone(),
        ))
    })
}
