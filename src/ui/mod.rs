mod browse;
mod chrome;
mod discover;
mod kit;
mod login;
mod messages;
mod rooms;
mod search;
mod settings;
mod transfers;
mod uploads;
mod users;

use gpui_kit::component::button::Button;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Sizable, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::HashSet;

use crate::config::{self, Config};
use crate::net::{Command, LoginSettings, Notice, NoticeLevel, Session, Status};
use crate::theme::{self, Mode, palette};

use browse::BrowseView;
use chrome::{Page, Presence};
use discover::{DiscoverEvent, DiscoverView};
use login::{LoginRequest, LoginView};
use messages::MessagesView;
use rooms::RoomsView;
use search::SearchView;
use settings::{SettingsEvent, SettingsView};
use transfers::TransfersView;
use uploads::{UploadsEvent, UploadsView};
use users::UsersView;

/// Something to do with a user, raised by any view that shows usernames.
#[derive(Clone)]
pub enum UserAction {
    Browse(String),
    Message(String),
    SearchUser(String),
    Info(String),
    SetBuddy(String, bool),
    SetIgnored(String, bool),
}

/// Buddy and ignore lists, readable wherever a user menu is built.
#[derive(Default)]
pub struct Social {
    pub buddies: HashSet<String>,
    pub ignored: HashSet<String>,
}

impl Global for Social {}

struct PendingLogin {
    username: String,
    password: String,
    remember: bool,
}

pub struct Workspace {
    session: Entity<Session>,
    config: Config,
    page: Page,
    status: Status,
    pending: Option<PendingLogin>,
    login: Entity<LoginView>,
    search: Entity<SearchView>,
    transfers: Entity<TransfersView>,
    uploads: Entity<UploadsView>,
    browse: Entity<BrowseView>,
    messages: Entity<MessagesView>,
    rooms: Entity<RoomsView>,
    users: Entity<UsersView>,
    discover: Entity<DiscoverView>,
    settings: Entity<SettingsView>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config = Config::load();
        theme::apply(&config.theme, config.mode, Some(window), cx);

        let session = cx.new(Session::new);
        let login = cx.new(|cx| LoginView::new(&config.username, config.remember, window, cx));
        let search = cx.new(|cx| SearchView::new(session.clone(), window, cx));
        let transfers =
            cx.new(|cx| TransfersView::new(session.clone(), config.download_dir.clone(), cx));
        let uploads = cx.new(|cx| {
            UploadsView::new(
                session.clone(),
                config.shared_dirs.clone(),
                config.upload_slots,
                window,
                cx,
            )
        });
        let browse = cx.new(|cx| BrowseView::new(session.clone(), window, cx));
        let messages = cx.new(|cx| MessagesView::new(session.clone(), window, cx));
        let rooms = cx.new(|cx| RoomsView::new(session.clone(), window, cx));
        let users = cx.new(|cx| UsersView::new(session.clone(), window, cx));
        let discover = cx.new(|cx| DiscoverView::new(session.clone(), window, cx));
        let settings = cx.new(|cx| SettingsView::new(&config, session.clone(), window, cx));

        let subscriptions = vec![
            cx.subscribe_in(&login, window, |this, _, request: &LoginRequest, _, cx| {
                this.start_login(
                    request.username.clone(),
                    request.password.clone(),
                    request.remember,
                    cx,
                );
            }),
            cx.subscribe_in(&settings, window, Self::on_settings),
            cx.subscribe_in(&uploads, window, Self::on_uploads),
            cx.subscribe_in(&search, window, Self::open_user),
            cx.subscribe_in(&transfers, window, Self::open_user),
            cx.subscribe_in(&uploads, window, Self::open_user),
            cx.subscribe_in(&messages, window, Self::open_user),
            cx.subscribe_in(&rooms, window, Self::open_user),
            cx.subscribe_in(&users, window, Self::open_user),
            cx.subscribe_in(&discover, window, Self::open_user),
            cx.subscribe_in(&discover, window, Self::on_discover),
            cx.subscribe_in(&session, window, |_, _, notice: &Notice, window, cx| {
                let note = match notice.0 {
                    NoticeLevel::Info => Notification::info(notice.1.clone()),
                    NoticeLevel::Warning => Notification::warning(notice.1.clone()),
                    NoticeLevel::Alert if window.is_window_active() => {
                        Notification::info(notice.1.clone())
                    }
                    NoticeLevel::Alert => Notification::info(notice.1.clone()).in_app_and_system(),
                };
                window.push_notification(note, cx);
            }),
            cx.observe_in(&session, window, Self::on_session),
            cx.observe_window_appearance(window, |this, window, cx| {
                if this.config.mode == Mode::System {
                    theme::apply(&this.config.theme, Mode::System, Some(window), cx);
                }
            }),
        ];

        let mut this = Self {
            session,
            config,
            page: Page::Search,
            status: Status::Offline,
            pending: None,
            login,
            search,
            transfers,
            uploads,
            browse,
            messages,
            rooms,
            users,
            discover,
            settings,
            _subscriptions: subscriptions,
        };
        this.sync_social(cx);
        this.auto_login(cx);
        this
    }

    fn auto_login(&mut self, cx: &mut Context<Self>) {
        if !self.config.remember || self.config.username.is_empty() {
            return;
        }
        if let Some(password) = config::saved_password(&self.config.username) {
            self.start_login(self.config.username.clone(), password, true, cx);
        }
    }

    fn start_login(
        &mut self,
        username: String,
        password: String,
        remember: bool,
        cx: &mut Context<Self>,
    ) {
        self.pending = Some(PendingLogin {
            username: username.clone(),
            password: password.clone(),
            remember,
        });
        let command = Command::Login(LoginSettings {
            username,
            password,
            listen_port: self.config.listen_port,
            download_dir: self.config.download_dir.clone(),
            shares: self.config.shared_dirs.clone(),
            upload_slots: self.config.upload_slots,
            buddies: self.config.buddies.clone(),
            likes: self.config.likes.clone(),
            dislikes: self.config.dislikes.clone(),
            upnp: self.config.upnp,
            download_limit: self.config.download_limit,
        });
        self.session
            .update(cx, |session, cx| session.login(command, cx));
    }

    fn on_session(
        &mut self,
        session: Entity<Session>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let status = session.read(cx).status.clone();
        if status == self.status {
            return;
        }
        let was_online = self.status.has_session();
        self.status = status.clone();
        self.login
            .update(cx, |login, cx| login.set_status(status.clone(), cx));

        match &status {
            Status::Online => {
                if let Some(pending) = self.pending.take() {
                    self.remember(pending, cx);
                }
                if !was_online {
                    self.page = Page::Search;
                    self.search
                        .update(cx, |search, cx| search.focus(window, cx));
                }
            }
            Status::Failed(_) | Status::Offline => self.pending = None,
            _ => {}
        }
        cx.notify();
    }

    fn remember(&mut self, pending: PendingLogin, cx: &mut Context<Self>) {
        self.config.username = pending.username.clone();
        self.config.remember = pending.remember;
        if pending.remember {
            config::store_password(&pending.username, &pending.password);
        } else {
            config::forget_password(&pending.username);
        }
        self.save();
        let username: SharedString = pending.username.into();
        self.settings
            .update(cx, |settings, _| settings.set_username(username));
    }

    fn on_settings(
        &mut self,
        _: &Entity<SettingsView>,
        event: &SettingsEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SettingsEvent::DownloadDir(dir) => {
                if *dir == self.config.download_dir {
                    return;
                }
                self.config.download_dir = dir.clone();
                self.session
                    .read(cx)
                    .send(Command::SetDownloadDir(dir.clone()));
                self.transfers
                    .update(cx, |view, _| view.set_download_dir(dir.clone()));
            }
            SettingsEvent::ListenPort(port) => self.config.listen_port = *port,
            SettingsEvent::Theme(id, mode) => {
                self.config.theme = id.to_string();
                self.config.mode = *mode;
                theme::apply(id, *mode, Some(window), cx);
            }
            SettingsEvent::Away(away) => {
                self.session
                    .update(cx, |session, cx| session.set_away(*away, cx));
            }
            SettingsEvent::Upnp(on) => {
                self.config.upnp = *on;
                self.session.read(cx).send(Command::SetUpnp(*on));
            }
            SettingsEvent::DownloadLimit(limit) => {
                self.config.download_limit = *limit;
                self.session
                    .read(cx)
                    .send(Command::SetDownloadLimit(*limit));
            }
            SettingsEvent::ChangePassword(password) => {
                self.session
                    .read(cx)
                    .send(Command::ChangePassword(password.clone()));
                if self.config.remember {
                    config::store_password(&self.config.username, password);
                }
            }
            SettingsEvent::Logout => {
                config::forget_password(&self.config.username);
                self.session.update(cx, |session, cx| session.logout(cx));
                self.login
                    .update(cx, |login, cx| login.clear_password(window, cx));
                self.page = Page::Search;
            }
        }
        self.save();
        cx.notify();
    }

    fn on_uploads(
        &mut self,
        _: &Entity<UploadsView>,
        event: &UploadsEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = self.session.read(cx);
        match event {
            UploadsEvent::Shares(shares) => {
                self.config.shared_dirs = shares.clone();
                session.send(Command::SetShares(shares.clone()));
            }
            UploadsEvent::Slots(slots) => {
                if *slots == self.config.upload_slots {
                    return;
                }
                self.config.upload_slots = *slots;
                session.send(Command::SetUploadSlots(*slots));
            }
        }
        self.save();
    }

    fn open_user<V>(
        &mut self,
        _: &Entity<V>,
        event: &UserAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            UserAction::Browse(username) => {
                self.browse
                    .update(cx, |browse, cx| browse.open(username, cx));
                self.select(Page::Browse, window, cx);
            }
            UserAction::Message(username) => {
                self.messages
                    .update(cx, |messages, cx| messages.open(username, window, cx));
                self.select(Page::Messages, window, cx);
            }
            UserAction::SearchUser(username) => {
                self.select(Page::Search, window, cx);
                self.search
                    .update(cx, |search, cx| search.prefill_user(username, window, cx));
            }
            UserAction::Info(username) => {
                self.session
                    .update(cx, |session, cx| session.look_up(username, cx));
                self.select(Page::Users, window, cx);
            }
            UserAction::SetBuddy(username, add) => {
                let list = &mut self.config.buddies;
                list.retain(|name| name != username);
                if *add {
                    list.push(username.clone());
                }
                let command = if *add {
                    Command::Watch(username.clone())
                } else {
                    Command::Unwatch(username.clone())
                };
                self.session.read(cx).send(command);
                self.save();
                self.sync_social(cx);
            }
            UserAction::SetIgnored(username, ignore) => {
                let list = &mut self.config.ignored;
                list.retain(|name| name != username);
                if *ignore {
                    list.push(username.clone());
                }
                self.save();
                self.sync_social(cx);
            }
        }
    }

    /// Pushes the saved buddy and ignore lists to the menus and the session.
    fn sync_social(&mut self, cx: &mut Context<Self>) {
        let ignored: HashSet<String> = self.config.ignored.iter().cloned().collect();
        cx.set_global(Social {
            buddies: self.config.buddies.iter().cloned().collect(),
            ignored: ignored.clone(),
        });
        let (likes, dislikes) = (self.config.likes.clone(), self.config.dislikes.clone());
        self.session.update(cx, |session, cx| {
            session.ignored = ignored;
            session.likes = likes;
            session.dislikes = dislikes;
            cx.notify();
        });
        cx.notify();
    }

    fn on_discover(
        &mut self,
        _: &Entity<DiscoverView>,
        event: &DiscoverEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            DiscoverEvent::Search(query) => {
                self.search.update(cx, |search, cx| search.open(query, cx));
                self.select(Page::Search, window, cx);
            }
            DiscoverEvent::SetInterest { item, like, add } => {
                let list = if *like {
                    &mut self.config.likes
                } else {
                    &mut self.config.dislikes
                };
                list.retain(|existing| existing != item);
                if *add {
                    list.push(item.clone());
                }
                self.session.read(cx).send(Command::SetInterest {
                    item: item.clone(),
                    like: *like,
                    add: *add,
                });
                self.save();
                self.sync_social(cx);
            }
        }
    }

    fn save(&self) {
        if let Err(err) = self.config.save() {
            eprintln!("could not save settings: {err}");
        }
    }

    fn select(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.page = page;
        self.messages.update(cx, |messages, cx| {
            messages.set_visible(page == Page::Messages, cx)
        });
        self.rooms
            .update(cx, |rooms, cx| rooms.set_visible(page == Page::Rooms, cx));
        match page {
            Page::Search => self
                .search
                .update(cx, |search, cx| search.focus(window, cx)),
            Page::Browse => self
                .browse
                .update(cx, |browse, cx| browse.focus(window, cx)),
            Page::Rooms => self.rooms.update(cx, |rooms, cx| rooms.focus(window, cx)),
            Page::Users => self.users.update(cx, |users, cx| users.focus(window, cx)),
            Page::Discover => self.discover.update(cx, |discover, cx| {
                discover.focus(window, cx);
                discover.refresh(cx);
            }),
            Page::Messages => self
                .messages
                .update(cx, |messages, cx| messages.focus(window, cx)),
            _ => {}
        }
        cx.notify();
    }

    fn banner(&self, cx: &mut Context<Self>) -> Option<Div> {
        let p = palette(cx);
        let (text, color, action) = match &self.status {
            Status::Reconnecting { attempt } => (
                format!("lost the server connection. reconnecting, attempt {attempt}…"),
                p.warning,
                false,
            ),
            Status::Displaced => (
                "you logged in from somewhere else, so the server closed this session.".to_string(),
                p.danger,
                true,
            ),
            _ => return None,
        };
        Some(
            div()
                .flex()
                .items_center()
                .gap_3()
                .px(px(40.))
                .py_2()
                .border_b_1()
                .border_color(p.border_weak)
                .bg(p.bg_weak)
                .child(kit::dot(color))
                .child(div().flex_1().text_color(p.text).child(text))
                .when(action, |this| {
                    this.child(
                        Button::new("reconnect")
                            .outline()
                            .small()
                            .label("log in here again")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.session.read(cx).send(Command::Reconnect)
                            })),
                    )
                }),
        )
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let session = self.session.read(cx);
        let signed_in = session.status.has_session();
        let presence = signed_in.then(|| Presence {
            username: session.username.clone(),
            online: session.status.is_online(),
            away: session.away,
        });
        let counts = [
            (Page::Search, session.searches.len()),
            (Page::Transfers, session.active_downloads()),
            (Page::Uploads, session.active_uploads()),
            (Page::Messages, session.chats.unread()),
            (Page::Rooms, session.rooms.unread()),
        ];

        let root = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.bg)
            .text_color(p.text)
            .text_size(px(13.))
            .child(chrome::title_bar(presence, cx));

        if !signed_in {
            return root.child(div().flex_1().min_h_0().child(self.login.clone()));
        }

        let entity = cx.entity().downgrade();
        let nav = chrome::sidebar(
            self.page,
            &counts,
            move |page, window, cx| {
                entity
                    .update(cx, |this, cx| this.select(page, window, cx))
                    .ok();
            },
            cx,
        );
        let content = match self.page {
            Page::Search => self.search.clone().into_any_element(),
            Page::Transfers => self.transfers.clone().into_any_element(),
            Page::Uploads => self.uploads.clone().into_any_element(),
            Page::Browse => self.browse.clone().into_any_element(),
            Page::Messages => self.messages.clone().into_any_element(),
            Page::Settings => self.settings.clone().into_any_element(),
            Page::Rooms => self.rooms.clone().into_any_element(),
            Page::Users => self.users.clone().into_any_element(),
            Page::Discover => self.discover.clone().into_any_element(),
        };

        root.child(
            div().flex_1().min_h_0().flex().child(nav).child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .children(self.banner(cx))
                    .child(div().flex_1().min_h_0().child(content)),
            ),
        )
    }
}
