use iced::widget::{button, column, container, row, scrollable, text, text_input, svg, Space, stack};
use iced::{Alignment, Element, Length, Task, Font, Subscription};
use iced::font::{Family, Weight, Stretch, Style};
use taskflow_core::db::{self, Database};
use taskflow_core::models::{SyncState, Task as LocalTask, TaskList};
use taskflow_core::google::oauth::{load_credentials, run_oauth_flow};
use taskflow_core::google::token::TokenManager;
use taskflow_core::google::tasks_api::GoogleTasksClient;
use taskflow_core::sync::engine::run_sync;
use taskflow_core::sync::recurrence::handle_recurring_task_completion;
use rusqlite::Connection;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

use crate::theme::{AppTheme, ColorTokens};
use crate::widgets::icons;

pub const FONT_INTER: Font = Font {
    family: Family::Name("Inter"),
    weight: Weight::Normal,
    stretch: Stretch::Normal,
    style: Style::Normal,
};

pub const FONT_MONO: Font = Font {
    family: Family::Name("JetBrains Mono"),
    weight: Weight::Normal,
    stretch: Stretch::Normal,
    style: Style::Normal,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActiveView {
    Today,
    Upcoming,
    List(String), // list UUID
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewFadeDirection {
    FadeOut,
    FadeIn,
    Idle,
}

pub struct TaskFlowApp {
    db: Database,
    theme: AppTheme,
    active_view: ActiveView,
    lists: Vec<TaskList>,
    tasks: Vec<LocalTask>,
    quick_add_text: String,
    syncing: bool,
    status_message: String,
    authenticated: bool,
    
    // Animation States
    completing_tasks: HashMap<String, f32>,
    new_tasks: HashMap<String, f32>,
    pending_view: Option<ActiveView>,
    view_fade_progress: f32,
    view_fade_direction: ViewFadeDirection,
    sync_rotation: f32,
    sync_success_progress: f32,
    empty_state_time: f32,

    // Polish / Command Palette States
    selected_task_id: Option<String>,
    command_palette_open: bool,
    command_palette_text: String,
    selected_palette_index: usize,
    sync_interval_mins: u32,

    // Error and Connection States
    keyring_error: Option<String>,
    offline: bool,
    token_revoked: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    Init,
    None,
    LoadedData(Result<(Vec<TaskList>, Vec<LocalTask>), String>),
    SelectView(ActiveView),
    ToggleComplete(String), // task_id
    QuickAddChanged(String),
    QuickAddSubmit,
    TriggerSync,
    SyncFinished(Result<taskflow_core::sync::engine::SyncReport, String>),
    Authenticate,
    AuthFinished(Result<(), String>),
    Logout,
    Tick(std::time::Instant),
    EventOccurred(iced::Event),
    CommandPaletteChanged(String),
    CommandPaletteSubmit,
    ToggleTheme,
    SetSyncInterval(u32),
    CloseRevocationModal,
}

pub fn run() -> iced::Result {
    iced::application("TaskFlow", TaskFlowApp::update, TaskFlowApp::view)
        .window(iced::window::Settings {
            size: iced::Size::new(900.0, 600.0),
            ..Default::default()
        })
        .theme(TaskFlowApp::theme)
        .subscription(TaskFlowApp::subscription)
        .run_with(TaskFlowApp::new)
}

impl TaskFlowApp {
    fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            iced::event::listen().map(Message::EventOccurred)
        ];
        if self.has_active_animations() {
            subs.push(iced::time::every(std::time::Duration::from_millis(16)).map(Message::Tick));
        }
        Subscription::batch(subs)
    }

    fn has_active_animations(&self) -> bool {
        !self.completing_tasks.is_empty()
            || !self.new_tasks.is_empty()
            || self.view_fade_direction != ViewFadeDirection::Idle
            || self.syncing
            || self.sync_success_progress > 0.0
            || (self.tasks.is_empty() && self.active_view != ActiveView::Settings)
    }

    fn new() -> (Self, Task<Message>) {
        let db = Database::new().unwrap_or_else(|_| Database::in_memory());
        
        let keyring_result = TokenManager::load_refresh_token();
        let mut keyring_error = None;
        let mut authenticated = false;
        
        match keyring_result {
            Ok(Some(_)) => {
                authenticated = true;
            }
            Ok(None) => {
                // Not authenticated yet, no keyring error
            }
            Err(e) => {
                keyring_error = Some(e);
            }
        }

        let app = Self {
            db: db.clone(),
            theme: AppTheme::Dark,
            active_view: ActiveView::Today,
            lists: Vec::new(),
            tasks: Vec::new(),
            quick_add_text: String::new(),
            syncing: false,
            status_message: if keyring_error.is_some() {
                "Keyring error occurred.".to_string()
            } else if authenticated {
                "Logged in. Hit Sync to refresh.".to_string()
            } else {
                "Not authenticated. Log in from settings.".to_string()
            },
            authenticated,
            completing_tasks: HashMap::new(),
            new_tasks: HashMap::new(),
            pending_view: None,
            view_fade_progress: 1.0,
            view_fade_direction: ViewFadeDirection::Idle,
            sync_rotation: 0.0,
            sync_success_progress: 0.0,
            empty_state_time: 0.0,
            selected_task_id: None,
            command_palette_open: false,
            command_palette_text: String::new(),
            selected_palette_index: 0,
            sync_interval_mins: 5,
            keyring_error,
            offline: false,
            token_revoked: false,
        };

        // Load custom fonts on startup
        let load_inter = iced::font::load(include_bytes!("../../../assets/fonts/Inter-Regular.ttf") as &[u8]);
        let load_mono = iced::font::load(include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf") as &[u8]);

        let init_task = Task::batch(vec![
            load_inter.map(|_| Message::None),
            load_mono.map(|_| Message::None),
            Task::done(Message::Init),
        ]);

        (app, init_task)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Init => {
                self.keyring_error = None;
                let keyring_result = TokenManager::load_refresh_token();
                match keyring_result {
                    Ok(Some(_)) => {
                        self.authenticated = true;
                    }
                    Ok(None) => {
                        self.authenticated = false;
                    }
                    Err(e) => {
                        self.keyring_error = Some(e);
                        return Task::none();
                    }
                }

                let db = self.db.clone();
                let active_view = self.active_view.clone();
                Task::perform(
                    async move {
                        let conn = db.connect().map_err(|e| e.to_string())?;
                        let lists = db::task_lists::get_all(&conn).map_err(|e| e.to_string())?;
                        let tasks = Self::load_tasks_for_view(&conn, &active_view)?;
                        Ok((lists, tasks))
                    },
                    Message::LoadedData,
                )
            }
            Message::None => Task::none(),
            Message::LoadedData(Ok((lists, tasks))) => {
                if self.view_fade_direction == ViewFadeDirection::Idle {
                    for t in &tasks {
                        if !self.tasks.iter().any(|old_t| old_t.id == t.id) {
                            if !self.tasks.is_empty() {
                                self.new_tasks.insert(t.id.clone(), 0.0);
                            }
                        }
                    }
                } else {
                    self.new_tasks.clear();
                }
                self.lists = lists;
                self.tasks = tasks;
                Task::none()
            }
            Message::LoadedData(Err(e)) => {
                self.status_message = format!("Error loading data: {}", e);
                Task::none()
            }
            Message::SelectView(view) => {
                if self.active_view == view {
                    return Task::none();
                }
                self.pending_view = Some(view);
                self.view_fade_direction = ViewFadeDirection::FadeOut;
                Task::none()
            }
            Message::ToggleComplete(id) => {
                if let Some(task) = self.tasks.iter().find(|t| t.id == id) {
                    if task.status != "completed" {
                        self.completing_tasks.insert(id.clone(), 0.0);
                        return Task::none();
                    }
                }
                
                let db = self.db.clone();
                let active_view = self.active_view.clone();
                Task::perform(
                    async move {
                        let conn = db.connect().map_err(|e| e.to_string())?;
                        if let Some(mut task) = db::tasks::get(&conn, &id).map_err(|e| e.to_string())? {
                            if task.status == "completed" {
                                task.status = "needsAction".to_string();
                                task.completed_at = None;
                            } else {
                                task.status = "completed".to_string();
                                task.completed_at = Some(chrono::Utc::now());
                            }
                            task.sync_state = SyncState::Pending;
                            task.updated_at = chrono::Utc::now();
                            db::tasks::update(&conn, &task).map_err(|e| e.to_string())?;

                            if task.status == "completed" {
                                let _ = handle_recurring_task_completion(&conn, &task);
                            }
                        }
                        let lists = db::task_lists::get_all(&conn).map_err(|e| e.to_string())?;
                        let tasks = Self::load_tasks_for_view(&conn, &active_view)?;
                        Ok((lists, tasks))
                    },
                    Message::LoadedData,
                )
            }
            Message::QuickAddChanged(text) => {
                self.quick_add_text = text;
                Task::none()
            }
            Message::QuickAddSubmit => {
                let raw_text = self.quick_add_text.trim().to_string();
                self.quick_add_text.clear();

                let (title, parsed_date, reminder_time) = taskflow_core::parser::parse_task_input(&raw_text);
                if title.is_empty() {
                    return Task::none();
                }

                let target_list_id = match &self.active_view {
                    ActiveView::List(id) => Some(id.clone()),
                    _ => self.lists.iter().find(|l| l.is_default).map(|l| l.id.clone())
                        .or_else(|| self.lists.first().map(|l| l.id.clone())),
                };

                let list_id = match target_list_id {
                    Some(id) => id,
                    None => {
                        self.status_message = "Create a task list first.".to_string();
                        return Task::none();
                    }
                };

                let active_view_for_due = self.active_view.clone();
                let due_date = parsed_date.or_else(|| match &active_view_for_due {
                    ActiveView::Today => Some(chrono::Local::now().date_naive()),
                    _ => if reminder_time.is_some() {
                        Some(chrono::Local::now().date_naive())
                    } else {
                        None
                    }
                });

                let db = self.db.clone();
                let active_view = self.active_view.clone();
                Task::perform(
                    async move {
                        let conn = db.connect().map_err(|e| e.to_string())?;
                        let new_task = LocalTask {
                             id: uuid::Uuid::new_v4().to_string(),
                             google_id: None,
                             list_id,
                             title,
                             notes: None,
                             status: "needsAction".to_string(),
                             due_date,
                             reminder_time,
                             parent_id: None,
                             position: None,
                             completed_at: None,
                             updated_at: chrono::Utc::now(),
                             google_updated_at: None,
                             sync_state: SyncState::Pending,
                             is_deleted: false,
                             recurrence_rule: None,
                        };
                        db::tasks::create(&conn, &new_task).map_err(|e| e.to_string())?;
                        
                        let lists = db::task_lists::get_all(&conn).map_err(|e| e.to_string())?;
                        let tasks = Self::load_tasks_for_view(&conn, &active_view)?;
                        Ok((lists, tasks))
                    },
                    Message::LoadedData,
                )
            }
            Message::TriggerSync => {
                if !self.authenticated {
                    self.status_message = "Authenticate first to sync.".to_string();
                    return Task::none();
                }
                if self.syncing {
                    return Task::none();
                }
                self.syncing = true;
                self.status_message = "Syncing with Google Tasks...".to_string();
                
                let db = self.db.clone();
                Task::perform(
                    async move {
                        let creds = load_credentials().map_err(|e| e.to_string())?;
                        let token_manager = TokenManager::new();
                        let mut client = GoogleTasksClient::new(creds, token_manager);
                        run_sync(&db, &mut client).await
                    },
                    Message::SyncFinished,
                )
            }
            Message::SyncFinished(Ok(report)) => {
                self.syncing = false;
                self.sync_success_progress = 1.0;
                self.offline = false;
                self.token_revoked = false;
                self.status_message = format!(
                    "Sync success! Pulled: {}, Pushed: {}, Deleted: {}",
                    report.tasks_pulled, report.tasks_pushed, report.tasks_deleted
                );
                let db = self.db.clone();
                let active_view = self.active_view.clone();
                Task::perform(
                    async move {
                        let conn = db.connect().map_err(|e| e.to_string())?;
                        let lists = db::task_lists::get_all(&conn).map_err(|e| e.to_string())?;
                        let tasks = Self::load_tasks_for_view(&conn, &active_view)?;
                        Ok((lists, tasks))
                    },
                    Message::LoadedData,
                )
            }
            Message::SyncFinished(Err(e)) => {
                self.syncing = false;
                self.sync_success_progress = 0.0;
                self.status_message = format!("Sync failed: {}", e);
                
                let e_lower = e.to_lowercase();
                if e_lower.contains("network request failed") 
                    || e_lower.contains("refresh request failed") 
                    || e_lower.contains("connect error") 
                    || e_lower.contains("dns error") 
                    || e_lower.contains("timeout") 
                    || e_lower.contains("temporary failure in name resolution")
                {
                    self.offline = true;
                } else if e_lower.contains("failed to refresh token") 
                    || e_lower.contains("invalid_grant") 
                    || e_lower.contains("token revoked")
                {
                    self.token_revoked = true;
                    self.authenticated = false;
                }
                Task::none()
            }
            Message::Authenticate => {
                self.status_message = "Redirecting to browser for authentication...".to_string();
                Task::perform(
                    async {
                        let (_access, _expires, refresh) = run_oauth_flow().await?;
                        if let Some(ref_token) = refresh {
                            TokenManager::save_refresh_token(&ref_token)?;
                        }
                        Ok(())
                    },
                    Message::AuthFinished,
                )
            }
            Message::AuthFinished(Ok(())) => {
                self.authenticated = true;
                self.token_revoked = false;
                self.offline = false;
                self.status_message = "Successfully authenticated! Run sync.".to_string();
                Task::done(Message::Init)
            }
            Message::AuthFinished(Err(e)) => {
                self.status_message = format!("Authentication failed: {}", e);
                let e_lower = e.to_lowercase();
                if e_lower.contains("keyring") {
                    self.keyring_error = Some(e.clone());
                }
                Task::none()
            }
            Message::Logout => {
                let mut token_manager = TokenManager::new();
                let _ = token_manager.clear();
                self.authenticated = false;
                self.token_revoked = false;
                self.status_message = "Logged out successfully.".to_string();
                Task::done(Message::Init)
            }
            Message::CloseRevocationModal => {
                self.token_revoked = false;
                self.offline = true;
                Task::none()
            }
            Message::Tick(_now) => {
                let dt = 0.016;
                
                // 1. Completing tasks
                let mut completed_task_ids = Vec::new();
                for (id, progress) in self.completing_tasks.iter_mut() {
                    *progress += dt / 0.35;
                    if *progress >= 1.0 {
                        completed_task_ids.push(id.clone());
                    }
                }
                
                let mut completion_task = Task::none();
                if !completed_task_ids.is_empty() {
                    let id = completed_task_ids[0].clone();
                    for cid in completed_task_ids {
                        self.completing_tasks.remove(&cid);
                    }
                    
                    let db = self.db.clone();
                    let active_view = self.active_view.clone();
                    completion_task = Task::perform(
                        async move {
                            let conn = db.connect().map_err(|e| e.to_string())?;
                            if let Some(mut task) = db::tasks::get(&conn, &id).map_err(|e| e.to_string())? {
                                if task.status == "completed" {
                                    task.status = "needsAction".to_string();
                                    task.completed_at = None;
                                } else {
                                    task.status = "completed".to_string();
                                    task.completed_at = Some(chrono::Utc::now());
                                }
                                task.sync_state = SyncState::Pending;
                                task.updated_at = chrono::Utc::now();
                                db::tasks::update(&conn, &task).map_err(|e| e.to_string())?;

                                if task.status == "completed" {
                                    let _ = handle_recurring_task_completion(&conn, &task);
                                }
                            }
                            let lists = db::task_lists::get_all(&conn).map_err(|e| e.to_string())?;
                            let tasks = Self::load_tasks_for_view(&conn, &active_view)?;
                            Ok((lists, tasks))
                        },
                        Message::LoadedData,
                    );
                }

                // 2. New tasks progress
                self.new_tasks.retain(|_, progress| {
                    *progress += dt / 0.20;
                    *progress < 1.0
                });

                // 3. View fade transitions
                match self.view_fade_direction {
                    ViewFadeDirection::FadeOut => {
                        self.view_fade_progress -= dt / 0.06;
                        if self.view_fade_progress <= 0.0 {
                            self.view_fade_progress = 0.0;
                            if let Some(target_view) = self.pending_view.take() {
                                self.active_view = target_view;
                                self.quick_add_text.clear();
                                self.view_fade_direction = ViewFadeDirection::FadeIn;
                                let db = self.db.clone();
                                let active_view = self.active_view.clone();
                                return Task::perform(
                                    async move {
                                        let conn = db.connect().map_err(|e| e.to_string())?;
                                        let lists = db::task_lists::get_all(&conn).map_err(|e| e.to_string())?;
                                        let tasks = Self::load_tasks_for_view(&conn, &active_view)?;
                                        Ok((lists, tasks))
                                    },
                                    Message::LoadedData,
                                );
                            }
                        }
                    }
                    ViewFadeDirection::FadeIn => {
                        self.view_fade_progress += dt / 0.06;
                        if self.view_fade_progress >= 1.0 {
                            self.view_fade_progress = 1.0;
                            self.view_fade_direction = ViewFadeDirection::Idle;
                        }
                    }
                    ViewFadeDirection::Idle => {}
                }

                // 4. Sync rotation
                if self.syncing {
                    self.sync_rotation = (self.sync_rotation + 360.0 * dt) % 360.0;
                } else {
                    self.sync_rotation = 0.0;
                }

                // 5. Sync success progress timer
                if self.sync_success_progress > 0.0 {
                    self.sync_success_progress -= dt / 0.40;
                    if self.sync_success_progress < 0.0 {
                        self.sync_success_progress = 0.0;
                    }
                }

                // 6. Empty state breathing
                if self.tasks.is_empty() && self.active_view != ActiveView::Settings {
                    self.empty_state_time = (self.empty_state_time + dt * (2.0 * std::f32::consts::PI / 3.0)) % (2.0 * std::f32::consts::PI);
                }

                completion_task
            }
            Message::EventOccurred(event) => {
                if let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) = event {
                    use iced::keyboard::key::Named;
                    use iced::keyboard::Key;

                    // 1. Ctrl+K -> Toggle command palette
                    if modifiers.control() && (matches!(key, Key::Character(ref s) if s.as_str() == "k") || matches!(key, Key::Character(ref s) if s.as_str() == "K")) {
                        if self.command_palette_open {
                            self.command_palette_open = false;
                            return Task::none();
                        } else {
                            self.command_palette_open = true;
                            self.command_palette_text.clear();
                            self.selected_palette_index = 0;
                            return text_input::focus(text_input::Id::new("command_palette_input"));
                        }
                    }

                    // 2. Escape when palette is open -> close it
                    if self.command_palette_open {
                        match key {
                            Key::Named(Named::Escape) => {
                                self.command_palette_open = false;
                                return Task::none();
                            }
                            Key::Named(Named::ArrowDown) => {
                                let matches = self.get_palette_matches();
                                if !matches.is_empty() {
                                    self.selected_palette_index = (self.selected_palette_index + 1) % matches.len().min(8);
                                }
                                return Task::none();
                            }
                            Key::Named(Named::ArrowUp) => {
                                let matches = self.get_palette_matches();
                                if !matches.is_empty() {
                                    if self.selected_palette_index == 0 {
                                        self.selected_palette_index = matches.len().min(8) - 1;
                                    } else {
                                        self.selected_palette_index -= 1;
                                    }
                                }
                                return Task::none();
                            }
                            Key::Named(Named::Enter) => {
                                let matches = self.get_palette_matches();
                                if self.selected_palette_index < matches.len() {
                                    let (_, msg, _) = &matches[self.selected_palette_index];
                                    self.command_palette_open = false;
                                    return self.update(msg.clone());
                                }
                                return Task::none();
                            }
                            _ => {}
                        }
                        return Task::none();
                    }

                    // 3. Normal app shortcuts (when palette is closed)
                    // Ctrl+N -> focus quick add
                    if modifiers.control() && (matches!(key, Key::Character(ref s) if s.as_str() == "n") || matches!(key, Key::Character(ref s) if s.as_str() == "N")) {
                        return text_input::focus(text_input::Id::new("quick_add_input"));
                    }

                    // Ctrl+S -> trigger sync
                    if modifiers.control() && (matches!(key, Key::Character(ref s) if s.as_str() == "s") || matches!(key, Key::Character(ref s) if s.as_str() == "S")) {
                        return self.update(Message::TriggerSync);
                    }

                    // Ctrl+1..9 -> jump to sidebar lists
                    if modifiers.control() {
                        if let Key::Character(ref s) = key {
                            if let Ok(num) = s.parse::<usize>() {
                                if num >= 1 && num <= 9 {
                                    if num - 1 < self.lists.len() {
                                        let list_id = self.lists[num - 1].id.clone();
                                        return self.update(Message::SelectView(ActiveView::List(list_id)));
                                    }
                                }
                            }
                        }
                    }

                    // J/K or Arrow keys -> task list navigation
                    let mut rendered_tasks = Vec::new();
                    match &self.active_view {
                        ActiveView::Today => {
                            let today_date = chrono::Utc::now().naive_utc().date();
                            let mut overdue = Vec::new();
                            let mut today_tasks = Vec::new();
                            let mut completed = Vec::new();
                            for t in &self.tasks {
                                if t.status == "completed" {
                                    completed.push(t);
                                } else if let Some(due) = t.due_date {
                                    if due < today_date {
                                        overdue.push(t);
                                    } else if due == today_date {
                                        today_tasks.push(t);
                                    }
                                } else {
                                    today_tasks.push(t);
                                }
                            }
                            rendered_tasks.extend(overdue);
                            rendered_tasks.extend(today_tasks);
                            rendered_tasks.extend(completed);
                        }
                        ActiveView::Upcoming => {
                            let mut date_groups: std::collections::BTreeMap<Option<chrono::NaiveDate>, Vec<&LocalTask>> = std::collections::BTreeMap::new();
                            for t in &self.tasks {
                                date_groups.entry(t.due_date).or_default().push(t);
                            }
                            for (_, tasks) in date_groups {
                                rendered_tasks.extend(tasks);
                            }
                        }
                        ActiveView::List(_) => {
                            let mut active = Vec::new();
                            let mut completed = Vec::new();
                            for t in &self.tasks {
                                if t.status == "completed" {
                                    completed.push(t);
                                } else {
                                    active.push(t);
                                }
                            }
                            rendered_tasks.extend(active);
                            rendered_tasks.extend(completed);
                        }
                        ActiveView::Settings => {}
                    }

                    let is_down = matches!(key, Key::Named(Named::ArrowDown))
                        || match &key {
                            Key::Character(ref s) => s.as_str() == "j" || s.as_str() == "J",
                            _ => false,
                        };
                    let is_up = matches!(key, Key::Named(Named::ArrowUp))
                        || match &key {
                            Key::Character(ref s) => s.as_str() == "k" || s.as_str() == "K",
                            _ => false,
                        };

                    if is_down {
                        if !rendered_tasks.is_empty() {
                            let next_idx = match &self.selected_task_id {
                                Some(id) => {
                                    if let Some(pos) = rendered_tasks.iter().position(|t| &t.id == id) {
                                        (pos + 1).min(rendered_tasks.len() - 1)
                                    } else {
                                        0
                                    }
                                }
                                None => 0,
                            };
                            self.selected_task_id = Some(rendered_tasks[next_idx].id.clone());
                        }
                    } else if is_up {
                        if !rendered_tasks.is_empty() {
                            let next_idx = match &self.selected_task_id {
                                Some(id) => {
                                    if let Some(pos) = rendered_tasks.iter().position(|t| &t.id == id) {
                                        pos.saturating_sub(1)
                                    } else {
                                        0
                                    }
                                }
                                None => 0,
                            };
                            self.selected_task_id = Some(rendered_tasks[next_idx].id.clone());
                        }
                    } else if matches!(key, Key::Named(Named::Escape)) {
                        self.selected_task_id = None;
                    } else if matches!(key, Key::Named(Named::Space) | Key::Named(Named::Enter)) {
                        if let Some(id) = &self.selected_task_id {
                            return self.update(Message::ToggleComplete(id.clone()));
                        }
                    }
                }
                Task::none()
            }
            Message::CommandPaletteChanged(text) => {
                self.command_palette_text = text;
                self.selected_palette_index = 0;
                Task::none()
            }
            Message::CommandPaletteSubmit => {
                let matches = self.get_palette_matches();
                if self.selected_palette_index < matches.len() {
                    let (_, msg, _) = &matches[self.selected_palette_index];
                    self.command_palette_open = false;
                    return self.update(msg.clone());
                }
                Task::none()
            }
            Message::ToggleTheme => {
                self.theme = match self.theme {
                    AppTheme::Dark => AppTheme::Light,
                    AppTheme::Light => AppTheme::Dark,
                };
                Task::none()
            }
            Message::SetSyncInterval(mins) => {
                self.sync_interval_mins = mins;
                Task::none()
            }
        }
    }

    fn load_tasks_for_view(conn: &Connection, view: &ActiveView) -> Result<Vec<LocalTask>, String> {
        match view {
            ActiveView::Today => {
                // Today view fetches tasks due today or overdue
                let mut stmt = conn.prepare(
                    "SELECT id, google_id, list_id, title, notes, status, due_date, reminder_time, parent_id, position, completed_at, updated_at, google_updated_at, sync_state, is_deleted, recurrence_rule 
                     FROM tasks 
                     WHERE (due_date <= date('now', 'localtime') OR due_date IS NULL) AND is_deleted = 0 AND status = 'needsAction'
                     UNION
                     SELECT id, google_id, list_id, title, notes, status, due_date, reminder_time, parent_id, position, completed_at, updated_at, google_updated_at, sync_state, is_deleted, recurrence_rule 
                     FROM tasks 
                     WHERE completed_at >= date('now', 'start of day') AND is_deleted = 0 AND status = 'completed'
                     ORDER BY status ASC, due_date ASC, title ASC"
                ).map_err(|e| e.to_string())?;
                
                let task_iter = stmt.query_map([], |row| {
                    let due_date_str: Option<String> = row.get(6)?;
                    let parsed_due_date = due_date_str
                        .and_then(|s| chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok());

                    let reminder_time_str: Option<String> = row.get(7)?;
                    let parsed_reminder_time = reminder_time_str
                        .and_then(|s| chrono::NaiveTime::parse_from_str(&s, "%H:%M:%S").ok());

                    let completed_at_str: Option<String> = row.get(10)?;
                    let completed_at = completed_at_str
                        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                        .map(|dt| dt.with_timezone(&Utc));

                    let updated_at_str: String = row.get(11)?;
                    let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now());

                    let google_updated_at_str: Option<String> = row.get(12)?;
                    let google_updated_at = google_updated_at_str
                        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                        .map(|dt| dt.with_timezone(&Utc));

                    let sync_state_str: String = row.get(13)?;
                    let sync_state = std::str::FromStr::from_str(&sync_state_str)
                        .unwrap_or(SyncState::Pending);

                    let is_deleted: i32 = row.get(14)?;
                    let recurrence_rule_str: Option<String> = row.get(15)?;
                    let recurrence_rule = recurrence_rule_str
                        .and_then(|s| serde_json::from_str(&s).ok());

                    Ok(LocalTask {
                        id: row.get(0)?,
                        google_id: row.get(1)?,
                        list_id: row.get(2)?,
                        title: row.get(3)?,
                        notes: row.get(4)?,
                        status: row.get(5)?,
                        due_date: parsed_due_date,
                        reminder_time: parsed_reminder_time,
                        parent_id: row.get(8)?,
                        position: row.get(9)?,
                        completed_at,
                        updated_at,
                        google_updated_at,
                        sync_state,
                        is_deleted: is_deleted != 0,
                        recurrence_rule,
                    })
                }).map_err(|e| e.to_string())?;

                let mut tasks = Vec::new();
                for t in task_iter {
                    tasks.push(t.map_err(|e| e.to_string())?);
                }
                Ok(tasks)
            }
            ActiveView::Upcoming => db::tasks::get_upcoming(conn, 7).map_err(|e| e.to_string()),
            ActiveView::List(id) => db::tasks::get_all_active_in_list(conn, id).map_err(|e| e.to_string()),
            ActiveView::Settings => Ok(Vec::new()),
        }
    }

    fn theme(&self) -> iced::Theme {
        iced::Theme::Dark
    }

    fn view(&self) -> Element<'_, Message> {
        let sidebar_colors = self.theme.colors();
        let mut colors = self.theme.colors();

        if let Some(ref err) = self.keyring_error {
            let error_content = column![
                svg(icons::settings())
                    .width(48)
                    .height(48)
                    .style(move |_, _| svg::Style { color: Some(colors.accent_danger) }),
                Space::with_height(16),
                text("Keyring Error")
                    .font(FONT_INTER)
                    .size(24)
                    .style(move |_| text::Style { color: Some(colors.text_primary) }),
                Space::with_height(12),
                text("TaskFlow needs a secure keyring service (e.g., gnome-keyring, kwallet) to save your credentials. Please install or unlock your keyring.")
                    .font(FONT_INTER)
                    .size(14)
                    .align_x(Alignment::Center)
                    .style(move |_| text::Style { color: Some(colors.text_secondary) }),
                Space::with_height(24),
                container(
                    text(err)
                        .font(FONT_MONO)
                        .size(12)
                        .style(move |_| text::Style { color: Some(colors.accent_danger) })
                )
                .padding(12)
                .style(move |_| container::Style {
                    background: Some(iced::Background::Color(colors.bg_base)),
                    border: iced::Border {
                        color: colors.border_subtle,
                        width: 1.0,
                        radius: 8.0.into(),
                    },
                    ..Default::default()
                }),
                Space::with_height(24),
                button(text("Retry Connection").font(FONT_INTER).size(14))
                    .on_press(Message::Init)
                    .padding([10, 20])
                    .style(move |_, _| button::Style {
                        background: Some(iced::Background::Color(colors.accent_primary)),
                        text_color: colors.bg_base,
                        border: iced::Border { radius: 8.0.into(), ..Default::default() },
                        ..Default::default()
                    })
            ]
            .align_x(Alignment::Center)
            .width(Length::Fixed(500.0));

            return container(
                container(error_content)
                    .padding(32)
                    .style(move |_| container::Style {
                        background: Some(iced::Background::Color(colors.bg_surface)),
                        border: iced::Border {
                            color: colors.border_subtle,
                            width: 1.0,
                            radius: 12.0.into(),
                        },
                        ..Default::default()
                    })
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .style(move |_| container::Style {
                background: Some(iced::Background::Color(colors.bg_base)),
                ..Default::default()
            })
            .into();
        }

        let main_opacity = self.view_fade_progress;
        if main_opacity < 1.0 {
            colors.text_primary = with_opacity(colors.text_primary, main_opacity);
            colors.text_secondary = with_opacity(colors.text_secondary, main_opacity);
            colors.accent_primary = with_opacity(colors.accent_primary, main_opacity);
            colors.accent_success = with_opacity(colors.accent_success, main_opacity);
            colors.accent_warning = with_opacity(colors.accent_warning, main_opacity);
            colors.accent_danger = with_opacity(colors.accent_danger, main_opacity);
            colors.bg_surface = with_opacity(colors.bg_surface, main_opacity);
            colors.bg_surface_hover = with_opacity(colors.bg_surface_hover, main_opacity);
            colors.border_subtle = with_opacity(colors.border_subtle, main_opacity);
        }

        // ------------------
        // SIDEBAR COMPONENT
        // ------------------
        let sidebar_header = text("TaskFlow")
            .size(22)
            .font(FONT_INTER)
            .style(move |_| text::Style { color: Some(sidebar_colors.text_primary) });

        let sidebar_today_btn = button(
            row![
                svg(icons::calendar()).width(16).height(16).style(move |_, _| svg::Style { color: Some(sidebar_colors.text_primary) }),
                Space::with_width(12),
                text("Today").font(FONT_INTER).size(14)
            ]
            .align_y(Alignment::Center)
        )
        .on_press(Message::SelectView(ActiveView::Today))
        .padding(10)
        .width(Length::Fill)
        .style(move |_, _| button::Style {
            background: Some(iced::Background::Color(
                if self.active_view == ActiveView::Today { sidebar_colors.bg_surface_hover } else { sidebar_colors.bg_surface }
            )),
            text_color: sidebar_colors.text_primary,
            border: iced::Border {
                color: if self.active_view == ActiveView::Today { sidebar_colors.accent_primary } else { sidebar_colors.border_subtle },
                width: 1.0,
                radius: 8.0.into(),
            },
            ..Default::default()
        });

        let sidebar_upcoming_btn = button(
            row![
                svg(icons::calendar()).width(16).height(16).style(move |_, _| svg::Style { color: Some(sidebar_colors.text_secondary) }),
                Space::with_width(12),
                text("Upcoming").font(FONT_INTER).size(14)
            ]
            .align_y(Alignment::Center)
        )
        .on_press(Message::SelectView(ActiveView::Upcoming))
        .padding(10)
        .width(Length::Fill)
        .style(move |_, _| button::Style {
            background: Some(iced::Background::Color(
                if self.active_view == ActiveView::Upcoming { sidebar_colors.bg_surface_hover } else { sidebar_colors.bg_surface }
            )),
            text_color: sidebar_colors.text_primary,
            border: iced::Border {
                color: if self.active_view == ActiveView::Upcoming { sidebar_colors.accent_primary } else { sidebar_colors.border_subtle },
                width: 1.0,
                radius: 8.0.into(),
            },
            ..Default::default()
        });

        let mut lists_col = column![
            text("LISTS")
                .size(11)
                .font(FONT_INTER)
                .style(move |_| text::Style { color: Some(sidebar_colors.text_secondary) }),
            Space::with_height(4)
        ]
        .spacing(6);

        for list in &self.lists {
            let list_id = list.id.clone();
            let is_selected = match &self.active_view {
                ActiveView::List(id) => id == &list_id,
                _ => false,
            };
            lists_col = lists_col.push(
                button(text(&list.title).font(FONT_INTER).size(14))
                    .on_press(Message::SelectView(ActiveView::List(list_id)))
                    .padding(8)
                    .width(Length::Fill)
                    .style(move |_, _| button::Style {
                        background: Some(iced::Background::Color(
                            if is_selected { sidebar_colors.bg_surface_hover } else { sidebar_colors.bg_surface }
                        )),
                        text_color: sidebar_colors.text_primary,
                        border: iced::Border {
                            color: if is_selected { sidebar_colors.accent_primary } else { sidebar_colors.border_subtle },
                            width: 1.0,
                            radius: 6.0.into(),
                        },
                        ..Default::default()
                    })
            );
        }

        // Sync button rotation and success checkmark
        let sync_icon = if self.sync_success_progress > 0.0 {
            svg(icons::check())
                .width(14)
                .height(14)
                .style(move |_, _| svg::Style { color: Some(sidebar_colors.accent_success) })
        } else {
            let rotation_radians = self.sync_rotation * (std::f32::consts::PI / 180.0);
            svg(icons::refresh())
                .rotation(rotation_radians)
                .width(14)
                .height(14)
                .style(move |_, _| svg::Style { color: Some(sidebar_colors.bg_base) })
        };

        let sync_text = if self.syncing {
            "Syncing..."
        } else if self.sync_success_progress > 0.0 {
            "Synced!"
        } else {
            "Sync Now"
        };

        let sidebar_bottom = column![
            button(
                row![
                    sync_icon,
                    Space::with_width(8),
                    text(sync_text).font(FONT_INTER).size(13)
                ]
                .align_y(Alignment::Center)
            )
            .on_press(Message::TriggerSync)
            .padding(10)
            .width(Length::Fill)
            .style(move |_, _| button::Style {
                background: Some(iced::Background::Color(
                    if self.sync_success_progress > 0.0 {
                        with_opacity(sidebar_colors.accent_success, 0.2)
                    } else {
                        sidebar_colors.accent_primary
                    }
                )),
                text_color: if self.sync_success_progress > 0.0 { sidebar_colors.accent_success } else { sidebar_colors.bg_base },
                border: iced::Border {
                    radius: 8.0.into(),
                    color: if self.sync_success_progress > 0.0 { sidebar_colors.accent_success } else { iced::Color::TRANSPARENT },
                    width: if self.sync_success_progress > 0.0 { 1.0 } else { 0.0 },
                },
                ..Default::default()
            }),
            Space::with_height(8),
            button(
                row![
                    svg(icons::settings()).width(14).height(14).style(move |_, _| svg::Style { color: Some(sidebar_colors.text_primary) }),
                    Space::with_width(8),
                    text("Settings").font(FONT_INTER).size(13)
                ]
                .align_y(Alignment::Center)
            )
            .on_press(Message::SelectView(ActiveView::Settings))
            .padding(10)
            .width(Length::Fill)
            .style(move |_, _| button::Style {
                background: Some(iced::Background::Color(
                    if self.active_view == ActiveView::Settings { sidebar_colors.bg_surface_hover } else { sidebar_colors.bg_surface }
                )),
                text_color: sidebar_colors.text_primary,
                border: iced::Border {
                    color: sidebar_colors.border_subtle,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..Default::default()
            })
        ];

        let sidebar_col = column![
            sidebar_header,
            Space::with_height(20),
            sidebar_today_btn,
            Space::with_height(8),
            sidebar_upcoming_btn,
            Space::with_height(24),
            scrollable(lists_col).height(Length::Fill),
            Space::with_height(16),
            sidebar_bottom
        ]
        .spacing(8)
        .width(220);

        let sidebar = container(sidebar_col)
            .padding(16)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(iced::Background::Color(sidebar_colors.bg_surface)),
                border: iced::Border {
                    color: sidebar_colors.border_subtle,
                    width: 1.0,
                    ..Default::default()
                },
                ..Default::default()
            });

        // ------------------
        // MAIN PANELS
        // ------------------
        let main_content: Element<'_, Message> = match &self.active_view {
            ActiveView::Settings => {
                let auth_block = if self.authenticated {
                    column![
                        text("Connected to Google Tasks").font(FONT_INTER).size(15).style(move |_| text::Style { color: Some(colors.accent_success) }),
                        Space::with_height(12),
                        button(text("Disconnect Google Account").font(FONT_INTER).size(14))
                            .on_press(Message::Logout)
                            .padding(12)
                            .style(move |_, _| button::Style {
                                background: Some(iced::Background::Color(colors.accent_danger)),
                                text_color: colors.text_primary,
                                border: iced::Border { radius: 8.0.into(), ..Default::default() },
                                ..Default::default()
                            })
                    ]
                } else {
                    column![
                        text("Synchronize tasks locally and with Google Cloud").font(FONT_INTER).size(14).style(move |_| text::Style { color: Some(colors.text_secondary) }),
                        Space::with_height(12),
                        button(text("Connect Google Account").font(FONT_INTER).size(14))
                            .on_press(Message::Authenticate)
                            .padding(12)
                            .style(move |_, _| button::Style {
                                background: Some(iced::Background::Color(colors.accent_primary)),
                                text_color: colors.bg_base,
                                border: iced::Border { radius: 8.0.into(), ..Default::default() },
                                ..Default::default()
                            })
                    ]
                };

                let theme_block = row![
                    text("App Theme").font(FONT_INTER).size(15).style(move |_| text::Style { color: Some(colors.text_primary) }),
                    Space::with_width(Length::Fill),
                    button(text(match self.theme {
                        AppTheme::Dark => "Dark Mode",
                        AppTheme::Light => "Light Mode",
                    }).font(FONT_INTER).size(13))
                    .on_press(Message::ToggleTheme)
                    .padding([8, 16])
                    .style(move |_, _| button::Style {
                        background: Some(iced::Background::Color(colors.bg_base)),
                        text_color: colors.text_primary,
                        border: iced::Border {
                            color: colors.border_subtle,
                            width: 1.0,
                            radius: 8.0.into(),
                        },
                        ..Default::default()
                    })
                ]
                .align_y(Alignment::Center);

                let mut sync_interval_row = row![
                    text("Sync Interval").font(FONT_INTER).size(15).style(move |_| text::Style { color: Some(colors.text_primary) }),
                    Space::with_width(Length::Fill),
                ].spacing(8).align_y(Alignment::Center);

                for mins in [1, 2, 5, 10, 15] {
                    let is_selected = self.sync_interval_mins == mins;
                    let mins_str = format!("{}m", mins);
                    sync_interval_row = sync_interval_row.push(
                        button(text(mins_str).font(FONT_INTER).size(12))
                            .on_press(Message::SetSyncInterval(mins))
                            .padding([6, 12])
                            .style(move |_, _| button::Style {
                                background: Some(iced::Background::Color(
                                    if is_selected { colors.accent_primary } else { colors.bg_base }
                                )),
                                text_color: if is_selected { colors.bg_base } else { colors.text_primary },
                                border: iced::Border {
                                    color: if is_selected { colors.accent_primary } else { colors.border_subtle },
                                    width: 1.0,
                                    radius: 6.0.into(),
                                },
                                ..Default::default()
                            })
                    );
                }

                column![
                    text("Settings").size(30).font(FONT_INTER).style(move |_| text::Style { color: Some(colors.text_primary) }),
                    Space::with_height(24),
                    container(
                        column![
                            text("Google Tasks Sync").size(18).font(FONT_INTER).style(move |_| text::Style { color: Some(colors.text_primary) }),
                            Space::with_height(16),
                            auth_block
                        ]
                    )
                    .padding(24)
                    .width(Length::Fill)
                    .style(move |_| container::Style {
                        background: Some(iced::Background::Color(colors.bg_surface)),
                        border: iced::Border {
                            color: colors.border_subtle,
                            width: 1.0,
                            radius: 12.0.into(),
                        },
                        ..Default::default()
                    }),
                    Space::with_height(16),
                    container(
                        column![
                            text("Preferences").size(18).font(FONT_INTER).style(move |_| text::Style { color: Some(colors.text_primary) }),
                            Space::with_height(20),
                            theme_block,
                            Space::with_height(16),
                            sync_interval_row
                        ]
                    )
                    .padding(24)
                    .width(Length::Fill)
                    .style(move |_| container::Style {
                        background: Some(iced::Background::Color(colors.bg_surface)),
                        border: iced::Border {
                            color: colors.border_subtle,
                            width: 1.0,
                            radius: 12.0.into(),
                        },
                        ..Default::default()
                    })
                ]
                .spacing(16)
                .padding(32)
                .into()
            }
            ActiveView::Today => {
                let today_date = Utc::now().naive_utc().date();
                
                // Segment tasks
                let mut overdue_tasks = Vec::new();
                let mut today_tasks = Vec::new();
                let mut completed_tasks = Vec::new();

                for t in &self.tasks {
                    if t.status == "completed" {
                        completed_tasks.push(t);
                    } else if let Some(due) = t.due_date {
                        if due < today_date {
                            overdue_tasks.push(t);
                        } else if due == today_date {
                            today_tasks.push(t);
                        }
                    } else {
                        // Task with no due date (treat as today/general in Today view)
                        today_tasks.push(t);
                    }
                }

                let mut task_list_col = column![].spacing(16);

                // Section 1: Overdue
                if !overdue_tasks.is_empty() {
                    let mut sec = column![
                        text("Overdue")
                            .font(FONT_INTER)
                            .size(13)
                            .style(move |_| text::Style { color: Some(colors.accent_danger) }),
                        Space::with_height(4)
                    ].spacing(8);

                    for t in overdue_tasks {
                        sec = sec.push(self.render_task_row(t, colors, true));
                    }
                    task_list_col = task_list_col.push(sec);
                }

                // Section 2: Today
                if !today_tasks.is_empty() {
                    let mut sec = column![
                        text("Today")
                            .font(FONT_INTER)
                            .size(13)
                            .style(move |_| text::Style { color: Some(colors.accent_primary) }),
                        Space::with_height(4)
                    ].spacing(8);

                    for t in today_tasks {
                        sec = sec.push(self.render_task_row(t, colors, false));
                    }
                    task_list_col = task_list_col.push(sec);
                }

                // Section 3: Completed Today
                if !completed_tasks.is_empty() {
                    let mut sec = column![
                        text("Completed")
                            .font(FONT_INTER)
                            .size(13)
                            .style(move |_| text::Style { color: Some(colors.accent_success) }),
                        Space::with_height(4)
                    ].spacing(8);

                    for t in completed_tasks {
                        sec = sec.push(self.render_task_row(t, colors, false));
                    }
                    task_list_col = task_list_col.push(sec);
                }

                if self.tasks.is_empty() {
                    task_list_col = task_list_col.push(self.render_empty_state("No tasks for today. Enjoy your free time!", colors));
                }

                let quick_add = self.render_quick_add(colors);

                column![
                    text("Today").size(30).font(FONT_INTER).style(move |_| text::Style { color: Some(colors.text_primary) }),
                    Space::with_height(16),
                    scrollable(task_list_col).height(Length::Fill),
                    Space::with_height(16),
                    quick_add,
                ]
                .spacing(8)
                .padding(32)
                .into()
            }
            ActiveView::Upcoming => {
                // Group upcoming tasks by date
                let mut date_groups: std::collections::BTreeMap<Option<chrono::NaiveDate>, Vec<&LocalTask>> = std::collections::BTreeMap::new();
                for t in &self.tasks {
                    date_groups.entry(t.due_date).or_default().push(t);
                }

                let mut task_list_col = column![].spacing(20);

                if date_groups.is_empty() {
                    task_list_col = task_list_col.push(self.render_empty_state("No tasks scheduled for the next 7 days.", colors));
                } else {
                    for (due_date, tasks) in date_groups {
                        let header_text = match due_date {
                            Some(date) => date.format("%A, %B %e").to_string(),
                            None => "No Due Date".to_string(),
                        };

                        let mut sec = column![
                            text(header_text)
                                .font(FONT_INTER)
                                .size(13)
                                .style(move |_| text::Style { color: Some(colors.accent_primary) }),
                            Space::with_height(4)
                        ].spacing(8);

                        for t in tasks {
                            sec = sec.push(self.render_task_row(t, colors, false));
                        }
                        task_list_col = task_list_col.push(sec);
                    }
                }

                column![
                    text("Upcoming").size(30).font(FONT_INTER).style(move |_| text::Style { color: Some(colors.text_primary) }),
                    Space::with_height(16),
                    scrollable(task_list_col).height(Length::Fill),
                ]
                .spacing(8)
                .padding(32)
                .into()
            }
            ActiveView::List(id) => {
                let view_title = self.lists.iter().find(|l| &l.id == id).map(|l| l.title.clone())
                    .unwrap_or_else(|| "List Tasks".to_string());

                let mut task_list_col = column![].spacing(8);

                // Split into active vs completed
                let mut active = Vec::new();
                let mut completed = Vec::new();
                for t in &self.tasks {
                    if t.status == "completed" {
                        completed.push(t);
                    } else {
                        active.push(t);
                    }
                }

                for t in active {
                    task_list_col = task_list_col.push(self.render_task_row(t, colors, false));
                }

                if !completed.is_empty() {
                    task_list_col = task_list_col.push(Space::with_height(12));
                    task_list_col = task_list_col.push(
                        text("Completed")
                            .font(FONT_INTER)
                            .size(13)
                            .style(move |_| text::Style { color: Some(colors.text_secondary) })
                    );
                    for t in completed {
                        task_list_col = task_list_col.push(self.render_task_row(t, colors, false));
                    }
                }

                if self.tasks.is_empty() {
                    task_list_col = task_list_col.push(self.render_empty_state("This list is empty. Add a task to get started!", colors));
                }

                let quick_add = self.render_quick_add(colors);

                column![
                    text(view_title).size(30).font(FONT_INTER).style(move |_| text::Style { color: Some(colors.text_primary) }),
                    Space::with_height(16),
                    scrollable(task_list_col).height(Length::Fill),
                    Space::with_height(16),
                    quick_add,
                ]
                .spacing(8)
                .padding(32)
                .into()
            }
        };

        // Status Bar
        let status_bar = container(
            text(&self.status_message)
                .size(12)
                .font(FONT_INTER)
                .style(move |_| text::Style { color: Some(colors.text_secondary) })
        )
        .padding(10)
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(colors.bg_surface)),
            border: iced::Border {
                color: colors.border_subtle,
                width: 1.0,
                ..Default::default()
            },
            ..Default::default()
        });

        let main_content_with_banner: Element<'_, Message> = if self.offline {
            let banner = container(
                row![
                    svg(icons::refresh())
                        .width(14)
                        .height(14)
                        .style(move |_, _| svg::Style { color: Some(colors.bg_base) }),
                    Space::with_width(8),
                    text("You're offline — changes will sync when connected")
                        .font(FONT_INTER)
                        .size(13)
                        .style(move |_| text::Style { color: Some(colors.bg_base) }),
                    Space::with_width(Length::Fill),
                    button(text("Retry Sync").font(FONT_INTER).size(11))
                        .on_press(Message::TriggerSync)
                        .padding([4, 10])
                        .style(move |_, _| button::Style {
                            background: Some(iced::Background::Color(colors.bg_base)),
                            text_color: colors.accent_warning,
                            border: iced::Border { radius: 4.0.into(), ..Default::default() },
                            ..Default::default()
                        })
                ]
                .align_y(Alignment::Center)
            )
            .padding(8)
            .width(Length::Fill)
            .style(move |_| container::Style {
                background: Some(iced::Background::Color(colors.accent_warning)),
                ..Default::default()
            });

            column![banner, main_content].width(Length::Fill).height(Length::Fill).into()
        } else {
            main_content
        };

        let main_layout = column![
            row![sidebar, container(main_content_with_banner).width(Length::Fill).height(Length::Fill)],
            status_bar
        ];

        let screen: Element<'_, Message> = if self.token_revoked {
            let modal_box = self.render_revocation_modal(colors);
            stack![
                container(main_layout)
                    .width(Length::Fill)
                    .height(Length::Fill),
                container(modal_box)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center)
                    .style(move |_| container::Style {
                        background: Some(iced::Background::Color(iced::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 0.6,
                        })),
                        ..Default::default()
                    })
            ]
            .into()
        } else if self.command_palette_open {
            let palette_box = self.render_command_palette(sidebar_colors);
            stack![
                container(main_layout)
                    .width(Length::Fill)
                    .height(Length::Fill),
                container(palette_box)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center)
                    .style(move |_| container::Style {
                        background: Some(iced::Background::Color(iced::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 0.6,
                        })),
                        ..Default::default()
                    })
            ]
            .into()
        } else {
            container(main_layout).into()
        };

        container(screen)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(iced::Background::Color(colors.bg_base)),
                ..Default::default()
            })
            .into()
    }

    fn render_revocation_modal(&self, colors: ColorTokens) -> Element<'_, Message> {
        let modal_content = column![
            svg(icons::settings())
                .width(40)
                .height(40)
                .style(move |_, _| svg::Style { color: Some(colors.accent_danger) }),
            Space::with_height(16),
            text("Google Account Disconnected")
                .font(FONT_INTER)
                .size(18)
                .style(move |_| text::Style { color: Some(colors.text_primary) }),
            Space::with_height(8),
            text("Your Google account was disconnected or the login session expired. Please sign in again to sync.")
                .font(FONT_INTER)
                .size(13)
                .align_x(Alignment::Center)
                .style(move |_| text::Style { color: Some(colors.text_secondary) }),
            Space::with_height(20),
            row![
                button(text("Sign In").font(FONT_INTER).size(13))
                    .on_press(Message::Authenticate)
                    .padding([8, 16])
                    .style(move |_, _| button::Style {
                        background: Some(iced::Background::Color(colors.accent_primary)),
                        text_color: colors.bg_base,
                        border: iced::Border { radius: 6.0.into(), ..Default::default() },
                        ..Default::default()
                    }),
                Space::with_width(12),
                button(text("Use Offline").font(FONT_INTER).size(13))
                    .on_press(Message::CloseRevocationModal)
                    .padding([8, 16])
                    .style(move |_, _| button::Style {
                        background: Some(iced::Background::Color(colors.bg_base)),
                        text_color: colors.text_primary,
                        border: iced::Border {
                            color: colors.border_subtle,
                            width: 1.0,
                            radius: 6.0.into(),
                        },
                        ..Default::default()
                    })
            ]
            .align_y(Alignment::Center)
        ]
        .align_x(Alignment::Center)
        .width(Length::Fixed(400.0));

        container(modal_content)
            .padding(24)
            .style(move |_| container::Style {
                background: Some(iced::Background::Color(colors.bg_surface)),
                border: iced::Border {
                    color: colors.border_subtle,
                    width: 1.0,
                    radius: 12.0.into(),
                },
                shadow: iced::Shadow {
                    color: iced::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.5 },
                    offset: iced::Vector::new(0.0, 10.0),
                    blur_radius: 20.0,
                },
                ..Default::default()
            })
            .into()
    }

    fn render_task_row<'a>(&'a self, task: &'a LocalTask, colors: ColorTokens, is_overdue: bool) -> Element<'a, Message> {
        let progress_completing = self.completing_tasks.get(&task.id).copied().unwrap_or(0.0);
        let is_completing = progress_completing > 0.0;
        let is_completed = task.status == "completed" || is_completing;

        let progress_new = self.new_tasks.get(&task.id).copied().unwrap_or(1.0);
        let is_new = progress_new < 1.0;

        let is_selected = Some(&task.id) == self.selected_task_id.as_ref();

        let mut row_opacity = 1.0;
        let mut vertical_padding = 14.0;
        let mut row_height = Length::Shrink;

        if is_completing {
            if progress_completing > 0.43 {
                let p = (progress_completing - 0.43) / 0.57;
                row_opacity = 1.0 - p;
                vertical_padding = 14.0 * (1.0 - p);
                row_height = Length::Fixed((50.0 * (1.0 - p)).max(0.0));
            }
        } else if is_new {
            row_opacity = progress_new;
            vertical_padding = 14.0 * progress_new;
            row_height = Length::Fixed(50.0 * progress_new);
        }

        let mut colors = colors;
        if row_opacity < 1.0 {
            colors.text_primary = with_opacity(colors.text_primary, row_opacity);
            colors.text_secondary = with_opacity(colors.text_secondary, row_opacity);
            colors.accent_primary = with_opacity(colors.accent_primary, row_opacity);
            colors.accent_success = with_opacity(colors.accent_success, row_opacity);
            colors.accent_warning = with_opacity(colors.accent_warning, row_opacity);
            colors.accent_danger = with_opacity(colors.accent_danger, row_opacity);
            colors.bg_surface = with_opacity(colors.bg_surface, row_opacity);
            colors.bg_surface_hover = with_opacity(colors.bg_surface_hover, row_opacity);
            colors.border_subtle = with_opacity(colors.border_subtle, row_opacity);
            colors.bg_base = with_opacity(colors.bg_base, row_opacity);
        }

        let border_color = if is_selected { colors.accent_primary } else { colors.border_subtle };
        let border_width = if is_selected { 2.0 } else if row_opacity > 0.0 { 1.0 } else { 0.0 };
        let background_color = if is_selected { colors.bg_surface_hover } else { colors.bg_surface };

        let task_id = task.id.clone();

        // Custom Circle Checkbox Icon button
        let check_icon = if is_completed {
            let color = if is_completing {
                let p = (progress_completing / 0.43).min(1.0);
                let r = colors.text_secondary.r + (colors.accent_success.r - colors.text_secondary.r) * p;
                let g = colors.text_secondary.g + (colors.accent_success.g - colors.text_secondary.g) * p;
                let b = colors.text_secondary.b + (colors.accent_success.b - colors.text_secondary.b) * p;
                let a = colors.text_secondary.a + (colors.accent_success.a - colors.text_secondary.a) * p;
                iced::Color { r, g, b, a }
            } else {
                colors.accent_success
            };
            svg(icons::check()).width(16).height(16).style(move |_, _| svg::Style { color: Some(color) })
        } else {
            svg(icons::square()).width(16).height(16).style(move |_, _| svg::Style { color: Some(colors.text_secondary) })
        };

        let check_btn = button(check_icon)
            .on_press(Message::ToggleComplete(task_id))
            .style(move |_, _| button::Style {
                background: Some(iced::Background::Color(background_color)),
                text_color: colors.text_secondary,
                border: iced::Border { radius: 100.0.into(), ..Default::default() },
                ..Default::default()
            });

        // Title and notes layout
        let display_title = if is_completing {
            let p = (progress_completing / 0.43).min(1.0);
            strikethrough_animated(&task.title, p)
        } else if is_completed {
            strikethrough_animated(&task.title, 1.0)
        } else {
            task.title.clone()
        };

        let mut text_layout = column![
            text(display_title)
                .font(FONT_INTER)
                .size(14)
                .style(move |_| text::Style {
                    color: Some(if is_completed { colors.text_secondary } else { colors.text_primary })
                })
        ];

        if let Some(ref notes) = task.notes {
            if !notes.trim().is_empty() {
                text_layout = text_layout.push(
                    text(notes)
                        .font(FONT_INTER)
                        .size(11)
                        .style(move |_| text::Style { color: Some(colors.text_secondary) })
                );
            }
        }

        let mut meta_row = row![].spacing(12).align_y(Alignment::Center);

        // Due date badge (with clock icon)
        if let Some(due) = task.due_date {
            let due_str = due.format("%b %e").to_string();
            let text_color = if is_completed {
                colors.text_secondary
            } else if is_overdue {
                colors.accent_danger
            } else {
                colors.text_secondary
            };

            meta_row = meta_row.push(
                row![
                    svg(icons::calendar()).width(12).height(12).style(move |_, _| svg::Style { color: Some(text_color) }),
                    Space::with_width(4),
                    text(due_str).font(FONT_MONO).size(11).style(move |_| text::Style { color: Some(text_color) })
                ]
                .align_y(Alignment::Center)
            );
        }

        // Reminder time badge
        if let Some(reminder) = task.reminder_time {
            let reminder_str = reminder.format("%I:%M %p").to_string();
            let text_color = if is_completed {
                colors.text_secondary
            } else {
                colors.accent_warning
            };

            meta_row = meta_row.push(
                row![
                    svg(icons::bell()).width(12).height(12).style(move |_, _| svg::Style { color: Some(text_color) }),
                    Space::with_width(4),
                    text(reminder_str).font(FONT_MONO).size(11).style(move |_| text::Style { color: Some(text_color) })
                ]
                .align_y(Alignment::Center)
            );
        }

        // Recurrence icon badge
        if task.recurrence_rule.is_some() {
            meta_row = meta_row.push(
                svg(icons::repeat())
                    .width(12)
                    .height(12)
                    .style(move |_, _| svg::Style {
                        color: Some(if is_completed { colors.text_secondary } else { colors.accent_primary })
                    })
            );
        }

        // List Chip (when in Today or Upcoming views)
        if self.active_view == ActiveView::Today || self.active_view == ActiveView::Upcoming {
            if let Some(list) = self.lists.iter().find(|l| l.id == task.list_id) {
                meta_row = meta_row.push(
                    container(
                        text(&list.title)
                            .font(FONT_INTER)
                            .size(10)
                            .style(move |_| text::Style { color: Some(colors.text_secondary) })
                    )
                    .padding([2, 6])
                    .style(move |_| container::Style {
                        background: Some(iced::Background::Color(colors.bg_base)),
                        border: iced::Border {
                            color: colors.border_subtle,
                            width: 1.0,
                            radius: 4.0.into(),
                        },
                        ..Default::default()
                    })
                );
            }
        }

        let row_padding = iced::Padding {
            top: vertical_padding,
            bottom: vertical_padding,
            left: 14.0,
            right: 14.0,
        };

        container(
            row![
                check_btn,
                Space::with_width(12),
                container(text_layout).width(Length::Fill),
                meta_row
            ]
            .align_y(Alignment::Center)
        )
        .padding(row_padding)
        .width(Length::Fill)
        .height(row_height)
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(background_color)),
            border: iced::Border {
                color: border_color,
                width: border_width,
                radius: 8.0.into(),
            },
            ..Default::default()
        })
        .into()
    }

    fn render_quick_add<'a>(&'a self, colors: ColorTokens) -> Element<'a, Message> {
        row![
            text_input("Add a task (e.g. Buy milk tomorrow at 5pm)...", &self.quick_add_text)
                .on_input(Message::QuickAddChanged)
                .on_submit(Message::QuickAddSubmit)
                .padding(12)
                .font(FONT_INTER)
                .style(move |_, _| text_input::Style {
                    background: iced::Background::Color(colors.bg_surface),
                    border: iced::Border {
                        color: colors.border_subtle,
                        width: 1.0,
                        radius: 8.0.into(),
                    },
                    value: colors.text_primary,
                    placeholder: colors.text_secondary,
                    selection: colors.accent_primary,
                    icon: colors.text_secondary,
                }),
            button(
                row![
                    svg(icons::plus()).width(12).height(12).style(move |_, _| svg::Style { color: Some(colors.bg_base) }),
                    Space::with_width(6),
                    text("Add").font(FONT_INTER).size(13)
                ]
                .align_y(Alignment::Center)
            )
            .on_press(Message::QuickAddSubmit)
            .padding(12)
            .style(move |_, _| button::Style {
                background: Some(iced::Background::Color(colors.accent_primary)),
                text_color: colors.bg_base,
                border: iced::Border { radius: 8.0.into(), ..Default::default() },
                ..Default::default()
            })
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
    }

    fn render_empty_state<'a>(&self, message: &'a str, colors: ColorTokens) -> Element<'a, Message> {
        let offset = (self.empty_state_time.sin() * 4.0) + 4.0;
        container(
            column![
                Space::with_height(offset),
                svg(icons::calendar())
                    .width(48)
                    .height(48)
                    .style(move |_, _| svg::Style { color: Some(colors.text_secondary) }),
                Space::with_height(16),
                text(message)
                    .font(FONT_INTER)
                    .size(14)
                    .style(move |_| text::Style { color: Some(colors.text_secondary) }),
                Space::with_height((8.0 - offset).max(0.0)),
            ]
            .align_x(Alignment::Center)
        )
        .padding(32)
        .width(Length::Fill)
        .align_x(Alignment::Center)
        .into()
    }

    fn get_palette_matches(&self) -> Vec<(String, Message, String)> {
        let query = self.command_palette_text.to_lowercase();
        let mut matches = Vec::new();

        let actions = vec![
            ("Sync Now".to_string(), Message::TriggerSync, "Action".to_string()),
            ("Go to Today View".to_string(), Message::SelectView(ActiveView::Today), "Action".to_string()),
            ("Go to Upcoming View".to_string(), Message::SelectView(ActiveView::Upcoming), "Action".to_string()),
            ("Go to Settings".to_string(), Message::SelectView(ActiveView::Settings), "Action".to_string()),
            ("Toggle Dark/Light Mode".to_string(), Message::ToggleTheme, "Action".to_string()),
        ];
        for (title, msg, category) in actions {
            if title.to_lowercase().contains(&query) {
                matches.push((title, msg, category));
            }
        }

        for list in &self.lists {
            let title = format!("Go to List: {}", list.title);
            if title.to_lowercase().contains(&query) {
                matches.push((title, Message::SelectView(ActiveView::List(list.id.clone())), "List".to_string()));
            }
        }

        for task in &self.tasks {
            let title = format!("Toggle: {}", task.title);
            if title.to_lowercase().contains(&query) {
                matches.push((title, Message::ToggleComplete(task.id.clone()), "Task".to_string()));
            }
        }

        matches
    }

    fn render_command_palette(&self, colors: ColorTokens) -> Element<'_, Message> {
        let matches = self.get_palette_matches();
        
        let search_input = text_input("Search tasks, lists, or actions...", &self.command_palette_text)
            .on_input(Message::CommandPaletteChanged)
            .on_submit(Message::CommandPaletteSubmit)
            .id(text_input::Id::new("command_palette_input"))
            .padding(14)
            .font(FONT_INTER)
            .style(move |_, _| text_input::Style {
                background: iced::Background::Color(colors.bg_base),
                border: iced::Border {
                    color: colors.border_subtle,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                value: colors.text_primary,
                placeholder: colors.text_secondary,
                selection: colors.accent_primary,
                icon: colors.text_secondary,
            });

        let mut results_col = column![].spacing(4);
        
        if matches.is_empty() {
            results_col = results_col.push(
                container(
                    text("No results found.")
                        .font(FONT_INTER)
                        .size(13)
                        .style(move |_| text::Style { color: Some(colors.text_secondary) })
                )
                .padding(12)
                .width(Length::Fill)
            );
        } else {
            for (idx, (title, msg, category)) in matches.into_iter().enumerate().take(8) {
                let is_selected = idx == self.selected_palette_index;
                
                let item_btn = button(
                    row![
                        container(
                            text(category)
                                .size(9)
                                .font(FONT_MONO)
                                .style(move |_| text::Style { color: Some(if is_selected { colors.bg_base } else { colors.text_secondary }) })
                        )
                        .padding([2, 6])
                        .style(move |_| container::Style {
                            background: Some(iced::Background::Color(if is_selected { colors.text_primary } else { colors.bg_base })),
                            border: iced::Border {
                                color: colors.border_subtle,
                                width: 1.0,
                                radius: 4.0.into(),
                            },
                            ..Default::default()
                        }),
                        Space::with_width(12),
                        text(title)
                            .size(13)
                            .font(FONT_INTER)
                            .style(move |_| text::Style { color: Some(colors.text_primary) })
                    ]
                    .align_y(Alignment::Center)
                )
                .on_press(msg)
                .padding(10)
                .width(Length::Fill)
                .style(move |_, _| button::Style {
                    background: Some(iced::Background::Color(
                        if is_selected { colors.bg_surface_hover } else { colors.bg_surface }
                    )),
                    text_color: colors.text_primary,
                    border: iced::Border {
                        color: if is_selected { colors.accent_primary } else { iced::Color::TRANSPARENT },
                        width: if is_selected { 1.0 } else { 0.0 },
                        radius: 8.0.into(),
                    },
                    ..Default::default()
                });
                
                results_col = results_col.push(item_btn);
            }
        }

        container(
            column![
                search_input,
                Space::with_height(12),
                scrollable(results_col).height(Length::Shrink)
            ]
            .spacing(4)
        )
        .padding(16)
        .width(Length::Fixed(500.0))
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(colors.bg_surface)),
            border: iced::Border {
                color: colors.border_subtle,
                width: 1.0,
                radius: 12.0.into(),
            },
            shadow: iced::Shadow {
                color: iced::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.5 },
                offset: iced::Vector::new(0.0, 10.0),
                blur_radius: 20.0,
            },
            ..Default::default()
        })
        .into()
    }
}

fn with_opacity(color: iced::Color, opacity: f32) -> iced::Color {
    iced::Color {
        a: color.a * opacity,
        ..color
    }
}

fn strikethrough_animated(s: &str, progress: f32) -> String {
    let char_count = s.chars().count();
    let strike_count = ((char_count as f32) * progress).round() as usize;
    s.chars().enumerate().map(|(i, c)| {
        if i < strike_count {
            format!("{}\u{0336}", c)
        } else {
            c.to_string()
        }
    }).collect()
}

