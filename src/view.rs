use std::sync::LazyLock;

use chrono::Utc;
use cosmic::applet::cosmic_panel_config::PanelAnchor;
use cosmic::iced::core::Alignment;
use cosmic::iced::{Color, Length};
use cosmic::theme::{Button, Text};
use cosmic::widget::{
    self, autosize, button, column, icon, progress_bar, row, segmented_button, text,
};
use cosmic::Element;

use crate::app::{AccountState, Message, OpenCodeGoApplet};
use crate::config::ICON_NAME;
use crate::usage::{FetchError, WindowQuota};

const WARNING_COLOR: Color = Color::from_rgb8(0xE6, 0x8A, 0x2E);
const DANGER_COLOR: Color = Color::from_rgb8(0xE0, 0x4F, 0x3F);

static AUTOSIZE_MAIN_ID: LazyLock<widget::Id> =
    LazyLock::new(|| widget::Id::new("opencode-go-autosize-main"));

/// The applet button shown in the panel: icon followed by one label per account.
pub fn panel(app: &OpenCodeGoApplet) -> Element<'_, Message> {
    let is_horizontal = matches!(
        app.core.applet.anchor,
        PanelAnchor::Top | PanelAnchor::Bottom
    );
    let suggested = app.core.applet.suggested_size(true);
    let padding = app.core.applet.suggested_padding(true);

    let mut children: Vec<Element<'_, Message>> = vec![icon::from_name(ICON_NAME)
        .size(suggested.0)
        .into()];
    for (index, state) in app.states.iter().enumerate() {
        if index > 0 {
            children.push(label_separator(is_horizontal, suggested));
        }
        children.push(account_label(app, state));
    }

    let content: Element<'_, Message> = if is_horizontal {
        row::with_children(children)
            .spacing(f32::from(padding.1))
            .align_y(Alignment::Center)
            .into()
    } else {
        column::with_children(children)
            .spacing(f32::from(padding.1))
            .align_x(Alignment::Center)
            .into()
    };

    // Padding is only applied along the panel's major axis, and the view is
    // wrapped in `autosize` so the applet window resizes to fit the content:
    // without it the window stays at the suggested icon size and the labels
    // get clipped (the applet is then clickable but shows nothing).
    let button = button::custom(content)
        .on_press_down(Message::TogglePopup)
        .class(Button::AppletIcon)
        .padding(if is_horizontal {
            [0.0, f32::from(padding.0)]
        } else {
            [f32::from(padding.0), 0.0]
        });

    autosize::autosize(button, AUTOSIZE_MAIN_ID.clone()).into()
}

/// Popup listing every configured account with its three quota windows.
pub fn popup(app: &OpenCodeGoApplet) -> Element<'_, Message> {
    let mut list = widget::list_column();

    if app.config.accounts.is_empty() {
        list = list.add(
            widget::settings::section().title("Accounts").add(
                widget::settings::item(
                    "No accounts configured yet",
                    button::standard("Add account").on_press(Message::ToggleContext),
                ),
            ),
        );
    } else {
        for (account, state) in app.config.accounts.iter().zip(&app.states) {
            let mut section = widget::settings::section().title(account.name.clone());
            match &state.usage {
                None => {
                    section = section.add(status_row(if state.fetching {
                        "Loading…"
                    } else {
                        "Not fetched yet"
                    }));
                }
                Some(Err(err)) => {
                    section = section.add(error_row(err));
                }
                Some(Ok(usage)) => {
                    section = section
                        .add(quota_item("5 hours", usage.rolling))
                        .add(quota_item("Weekly", usage.weekly))
                        .add(quota_item("Monthly", usage.monthly));
                }
            }
            list = list.add(section);
        }
    }

    let footer = row::with_children(vec![
        button::standard("Refresh").on_press(Message::Refresh).into(),
        button::standard("Settings")
            .on_press(Message::ToggleContext)
            .into(),
    ])
    .spacing(8)
    .padding([8, 12])
    .width(Length::Fill);

    app.core
        .applet
        .popup_container(column::with_children(vec![list.into(), footer.into()]).width(Length::Fill))
        .into()
}

/// Settings popup: manage accounts and the refresh interval.
pub fn context(app: &OpenCodeGoApplet) -> Element<'_, Message> {
    let mut list = widget::list_column();

    let mut accounts_section = widget::settings::section().title("Accounts");
    for (index, account) in app.config.accounts.iter().enumerate() {
        let name_input = widget::text_input("Name", &account.name)
            .on_input(move |value| Message::AccountName(index, value))
            .width(Length::Fill);
        let key_input = widget::text_input("API key", &account.key)
            .on_input(move |value| Message::AccountKey(index, value))
            .password()
            .width(Length::Fill);
        let remove = button::icon(icon::from_name("user-trash-symbolic"))
            .on_press(Message::RemoveAccount(index));
        accounts_section = accounts_section.add(
            column::with_children(vec![
                row::with_children(vec![name_input.into(), remove.into()]).spacing(8).into(),
                key_input.into(),
            ])
            .spacing(4),
        );
    }
    if app.config.accounts.is_empty() {
        accounts_section = accounts_section
            .add(status_row("No accounts configured yet. Add one below with your OpenCode Go API key."));
    }
    list = list.add(accounts_section);

    // The segmented control defaults to `width: Fill`; inside the shrink-to-fit
    // control slot of `settings::item` it would collapse to a sliver, so it
    // gets its own full-width row instead.
    list = list.add(widget::settings::section().title("General").add(
        column::with_children(vec![
            text::caption("Refresh interval").into(),
            segmented_button::horizontal(&app.interval_model)
                .on_activate(Message::IntervalSelected)
                .width(Length::Fill)
                .into(),
        ])
        .spacing(4)
        .width(Length::Fill),
    ));

    let footer = row::with_children(vec![
        button::standard("Add account")
            .on_press(Message::AddAccount)
            .into(),
        button::standard("Done")
            .on_press(Message::ToggleContext)
            .into(),
    ])
    .spacing(8)
    .padding([8, 12])
    .width(Length::Fill);

    app.core
        .applet
        .popup_container(column::with_children(vec![list.into(), footer.into()]).width(Length::Fill))
        .into()
}

/// A thin themed line separating account labels, reading like a `|`.
fn label_separator(is_horizontal: bool, suggested: (u16, u16)) -> Element<'static, Message> {
    let rule = if is_horizontal {
        Element::<'static, Message>::from(widget::divider::vertical::default())
    } else {
        Element::<'static, Message>::from(widget::divider::horizontal::default())
    };
    widget::container(rule)
        .width(Length::Fixed(if is_horizontal { 1.0 } else { f32::from(suggested.0) }))
        .height(Length::Fixed(if is_horizontal { f32::from(suggested.1) } else { 1.0 }))
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .into()
}

fn account_label<'a>(
    app: &'a OpenCodeGoApplet,
    state: &'a AccountState,
) -> Element<'a, Message> {
    let suggested = app.core.applet.suggested_size(true);
    let (content, color) = match &state.usage {
        None => ("…".to_string(), None),
        Some(Err(_)) => ("!".to_string(), Some(DANGER_COLOR)),
        Some(Ok(usage)) if usage.blocked() => ("blocked".to_string(), Some(DANGER_COLOR)),
        Some(Ok(usage)) => {
            let remaining = usage.worst_remaining().unwrap_or(100.0);
            let color = (remaining < 20.0).then_some(WARNING_COLOR);
            (format!("{:.0}%", remaining.round()), color)
        }
    };

    let mut label = app
        .core
        .applet
        .text(content)
        .font(cosmic::font::default())
        .height(Length::Fixed(f32::from(suggested.1)))
        .align_x(Alignment::Center)
        .align_y(Alignment::Center);
    if let Some(color) = color {
        label = label.class(Text::Color(color));
    }
    label.into()
}

#[allow(clippy::cast_possible_truncation)]
fn quota_item<'a>(label: &'static str, quota: Option<WindowQuota>) -> Element<'a, Message> {
    // Fixed width so every quota row's bar occupies identical bounds; the
    // control slot of `settings::item` shrinks to the natural content width,
    // which would otherwise misalign bars whenever a reset caption is long.
    const CONTROL_WIDTH: f32 = 180.0;

    let control: Element<'a, Message> = match quota {
        None => widget::container(text("—"))
            .width(Length::Fixed(CONTROL_WIDTH))
            .align_x(Alignment::Start)
            .into(),
        Some(quota) => {
            let remaining = quota.remaining_percent();
            let color = if quota.blocked() {
                Some(DANGER_COLOR)
            } else if remaining < 20.0 {
                Some(WARNING_COLOR)
            } else {
                None
            };

            let mut percent_text = text(format!("{:.0}%", remaining.round()));
            if let Some(color) = color {
                percent_text = percent_text.class(Text::Color(color));
            }

            // The bar shows the used quota: full (red) when exhausted, empty
            // when untouched; the number keeps showing what remains.
            let mut controls = column::with_children(vec![
                percent_text.into(),
                progress_bar::determinate_linear((quota.used_percent / 100.0) as f32)
                    .width(Length::Fill)
                    .into(),
            ])
            .spacing(4);

            let resets = resets_text(&quota);
            if !resets.is_empty() {
                controls = controls.push(text::caption(resets));
            }
            controls.width(Length::Fixed(CONTROL_WIDTH)).into()
        }
    };

    widget::settings::item(label, control).into()
}

fn resets_text(quota: &WindowQuota) -> String {
    let Some(resets_at) = quota.resets_at else {
        return String::new();
    };
    let now = Utc::now();
    if resets_at <= now {
        return "resets now".to_string();
    }
    let minutes = (resets_at - now).num_minutes();
    let prefix = if quota.rate_limited {
        "rate-limited — resets in "
    } else {
        "resets in "
    };
    if minutes >= 1440 {
        format!("{prefix}{}d {}h", minutes / 1440, (minutes % 1440) / 60)
    } else if minutes >= 60 {
        format!("{prefix}{}h {}m", minutes / 60, minutes % 60)
    } else {
        format!("{prefix}{}m", minutes.max(1))
    }
}

fn status_row(message: &str) -> Element<'_, Message> {
    cosmic::applet::padded_control(text(message).width(Length::Fill)).into()
}

fn error_row(error: &FetchError) -> Element<'_, Message> {
    cosmic::applet::padded_control(
        text(error.to_string())
            .class(Text::Color(DANGER_COLOR))
            .width(Length::Fill),
    )
    .into()
}
