// SPDX-License-Identifier: MPL-2.0

use std::time::Duration;

use cosmic::cosmic_config::{ConfigSet, CosmicConfigEntry};
use cosmic::iced::futures::SinkExt;
use cosmic::app::{Core, Task};
use cosmic::iced::platform_specific::shell::wayland::commands::popup::{destroy_popup, get_popup};
use cosmic::iced::window;
use cosmic::iced::{Limits, Subscription};
use cosmic::widget::segmented_button;
use cosmic::{Element, widget};

use crate::config::{Account, Config, APP_ID};
use crate::usage::{self, FetchError, Usage};
use crate::view;

/// Refresh interval choices offered in the settings, as `(seconds, label)`.
const INTERVALS: &[(u64, &str)] = &[
    (30, "30 s"),
    (60, "1 min"),
    (120, "2 min"),
    (300, "5 min"),
    (600, "10 min"),
];

#[derive(Clone, Debug, Default)]
pub struct AccountState {
    pub usage: Option<Result<Usage, FetchError>>,
    pub fetching: bool,
}

pub struct OpenCodeGoApplet {
    pub core: Core,
    pub popup: Option<window::Id>,
    pub context: Option<window::Id>,
    pub config: Config,
    /// Per-account fetch results, indexed like `config.accounts`.
    pub states: Vec<AccountState>,
    pub interval_model: segmented_button::Model<segmented_button::SingleSelect>,
}

#[derive(Debug, Clone)]
pub enum Message {
    TogglePopup,
    PopupClosed(window::Id),
    ToggleContext,
    ContextClosed(window::Id),
    UpdateConfig(Config),
    Refresh,
    Refreshed(Vec<(usize, Result<Usage, FetchError>)>),
    AccountName(usize, String),
    AccountKey(usize, String),
    AddAccount,
    RemoveAccount(usize),
    IntervalSelected(segmented_button::Entity),
}

fn build_interval_model(
    current: u64,
) -> segmented_button::Model<segmented_button::SingleSelect> {
    let mut builder = segmented_button::Model::<segmented_button::SingleSelect>::builder();
    for (secs, label) in INTERVALS {
        builder = builder.insert(|b| b.text(*label).data(*secs));
    }
    let model = builder.build();
    let position = INTERVALS
        .iter()
        .position(|(secs, _)| *secs == current)
        .unwrap_or(1);
    let mut model = model;
    model.activate_position(u16::try_from(position).unwrap_or(0));
    model
}

impl OpenCodeGoApplet {
    fn sync_states(&mut self) {
        self.states
            .resize_with(self.config.accounts.len(), AccountState::default);
    }

    fn persist_config(&self) {
        let Ok(handler) = cosmic::cosmic_config::Config::new(APP_ID, Config::VERSION) else {
            return;
        };
        if let Err(err) = handler.set("accounts", self.config.accounts.clone()) {
            eprintln!("failed to save accounts: {err}");
        }
        if let Err(err) = handler.set("refresh_secs", self.config.refresh_secs) {
            eprintln!("failed to save refresh interval: {err}");
        }
    }

    fn start_refresh(&mut self) -> Task<Message> {
        self.sync_states();
        if self.config.accounts.is_empty() {
            return Task::none();
        }
        for state in &mut self.states {
            state.fetching = true;
        }
        let accounts: Vec<(usize, String)> = self
            .config
            .accounts
            .iter()
            .enumerate()
            .map(|(index, account)| (index, account.key.clone()))
            .collect();

        cosmic::task::future(async move {
            let client = usage::client();
            let results = cosmic::iced::futures::future::join_all(
                accounts.into_iter().map(|(index, key)| {
                    let client = client.clone();
                    async move { (index, usage::fetch_usage(&client, &key).await) }
                }),
            )
            .await;
            Message::Refreshed(results)
        })
    }

    fn popup_task(&self, id: window::Id, min_width: f32, max_width: f32) -> Task<Message> {
        let mut popup_settings = self.core.applet.get_popup_settings(
            self.core.main_window_id().unwrap(),
            id,
            None,
            None,
            None,
        );
        popup_settings.positioner.size_limits = Limits::NONE
            .min_width(min_width)
            .max_width(max_width)
            .min_height(100.0)
            .max_height(1080.0);
        get_popup(popup_settings)
    }
}

impl cosmic::Application for OpenCodeGoApplet {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: Self::Flags) -> (Self, Task<Message>) {
        let config = Config::load();
        let interval_model = build_interval_model(config.refresh_secs);
        let mut app = Self {
            core,
            popup: None,
            context: None,
            config,
            states: Vec::new(),
            interval_model,
        };
        app.sync_states();
        (app, Task::none())
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        if Some(&id) == self.popup.as_ref() {
            Some(Message::PopupClosed(id))
        } else if Some(&id) == self.context.as_ref() {
            Some(Message::ContextClosed(id))
        } else {
            None
        }
    }

    fn update(&mut self, message: Self::Message) -> Task<Message> {
        match message {
            Message::TogglePopup => {
                return if let Some(popup) = self.popup.take() {
                    destroy_popup(popup)
                } else {
                    let new_id = window::Id::unique();
                    self.popup.replace(new_id);
                    let popup = self.popup_task(new_id, 360.0, 360.0);
                    Task::batch([popup, self.start_refresh()])
                };
            }
            Message::PopupClosed(id) => {
                if self.popup.as_ref() == Some(&id) {
                    self.popup = None;
                }
            }
            Message::ToggleContext => {
                let mut tasks: Vec<Task<Message>> = Vec::new();
                if let Some(popup) = self.popup.take() {
                    tasks.push(destroy_popup(popup));
                }
                if let Some(context) = self.context.take() {
                    tasks.push(destroy_popup(context));
                } else {
                    let new_id = window::Id::unique();
                    self.context.replace(new_id);
                    tasks.push(self.popup_task(new_id, 340.0, 440.0));
                }
                return Task::batch(tasks);
            }
            Message::ContextClosed(id) => {
                if self.context.as_ref() == Some(&id) {
                    self.context = None;
                }
            }
            Message::UpdateConfig(config) => {
                let interval_changed = config.refresh_secs != self.config.refresh_secs;
                self.config = config;
                if interval_changed {
                    self.interval_model = build_interval_model(self.config.refresh_secs);
                }
                self.sync_states();
            }
            Message::Refresh => {
                return self.start_refresh();
            }
            Message::Refreshed(results) => {
                for (index, result) in results {
                    if let Some(state) = self.states.get_mut(index) {
                        state.usage = Some(result);
                        state.fetching = false;
                    }
                }
            }
            Message::AccountName(index, name) => {
                if let Some(account) = self.config.accounts.get_mut(index) {
                    account.name = name;
                }
                self.persist_config();
            }
            Message::AccountKey(index, key) => {
                if let Some(account) = self.config.accounts.get_mut(index) {
                    account.key = key;
                }
                self.persist_config();
                return self.start_refresh();
            }
            Message::AddAccount => {
                let name = format!("Account {}", self.config.accounts.len() + 1);
                self.config.accounts.push(Account {
                    name,
                    key: String::new(),
                });
                self.sync_states();
                self.persist_config();
            }
            Message::RemoveAccount(index) => {
                self.config.accounts.remove(index);
                self.states.remove(index);
                self.persist_config();
            }
            Message::IntervalSelected(entity) => {
                if let Some(&secs) = self.interval_model.data::<u64>(entity)
                    && secs != self.config.refresh_secs
                {
                    self.config.refresh_secs = secs;
                    self.persist_config();
                }
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        view::panel(self)
    }

    fn view_window(&self, id: window::Id) -> Element<'_, Self::Message> {
        if Some(&id) == self.popup.as_ref() {
            view::popup(self)
        } else if Some(&id) == self.context.as_ref() {
            view::context(self)
        } else {
            widget::text("").into()
        }
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        let refresh_secs = self.config.refresh_secs.max(5);
        Subscription::batch([
            Subscription::run_with(refresh_secs, |secs: &u64| refresh_stream(*secs)),
            self.core
                .watch_config::<Config>(APP_ID)
                .map(|update| Message::UpdateConfig(update.config)),
        ])
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

fn refresh_stream(refresh_secs: u64) -> impl cosmic::iced::futures::Stream<Item = Message> {
    cosmic::iced::stream::channel(
        4,
        move |mut output: cosmic::iced::futures::channel::mpsc::Sender<Message>| async move {
            loop {
                let _ = output.send(Message::Refresh).await;
                tokio::time::sleep(Duration::from_secs(refresh_secs.max(1))).await;
            }
        },
    )
}
