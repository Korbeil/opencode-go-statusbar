mod app;
mod config;
mod usage;
mod view;

fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<app::OpenCodeGoApplet>(())
}
