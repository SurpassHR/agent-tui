use crate::components::*;
#[allow(unused_imports)]
use crate::state::*;
use crate::theme::Theme;
use crossterm::event::KeyCode;

impl Default for TuiState {
    fn default() -> Self {
        Self::new()
    }
}

impl TuiState {
    pub fn new() -> Self {
        Self {
            active_session: String::new(),
            current_time: String::new(),
            theme: Theme::cyan(),
            sidebar: Sidebar::default(),
            main_view: MainView::default(),
            agent_panel: AgentPanel::default(),
            bottom_bar: BottomBar::default(),
            popup: Popup::default(),
            workspaces: Vec::new(),
            focus_panel: FocusPanel::MainView,
            sidebar_cursor: 0,
            active_session_cursor: 0,
            last_enter_session: None,
            sidebar_subsection: SidebarSubsection::Workspace,
            main_view_subsection: MainViewSubsection::Input,
            agent_panel_subsection: AgentPanelSubsection::Agents,
            agent_cursor: 0,
            message_cursor: 0,
            scroll_mode: ScrollMode::TailFollow,
            selection: SelectionState::default(),
            shared_config: None,
            subagents: Vec::new(),
            skills: Vec::new(),
            skill_cursor: 0,
            mcps: Vec::new(),
            mcp_cursor: 0,
            router_running: false,
            router_port: 8001,
            providers: Vec::new(),
            current_model: String::new(),
            provider_cursor: 0,
            model_cursor: 0,
            model_search: String::new(),
            model_just_switched: false,
            active_provider_idx: None,
            provider_popup: None,
            provider_editor: None,
            models_fetch_rx: None,
            persistence_disabled: false,
            pending_ai_prompt: None,
            workspace_rename: None,
            rename_input: String::new(),
            confirm_delete: None,
            session_names: std::collections::HashMap::new(),
            snapshot_pending: false,
        }
    }

    /// 计算侧边栏工作区子区的可选项总数（0 = WORKSPACE title，1..=N = 树节点）
    pub fn sidebar_visible_count(&self) -> usize {
        let mut count = 1; // WORKSPACE title
        for ws in &self.workspaces {
            count += 1; // workspace header
            if ws.expanded {
                count += ws.sessions.len();
            }
        }
        count
    }

    /// 打开已有 Provider 编辑表单，预填充数据。
    fn open_edit_provider_editor(&mut self, idx: usize) {
        if let Some(p) = self.providers.get(idx) {
            self.provider_popup = None;
            let models_text = p
                .models
                .iter()
                .map(|m| format!("{}:{}:{}", m.id, m.tier, m.context_window))
                .collect::<Vec<_>>()
                .join("\n");
            self.provider_editor = Some(ProviderEditor {
                is_new: false,
                index: idx,
                draft: p.clone(),
                field_focus: 6,
                models_text,
                models_fetching: false,
                model_mgr: None,
            });
        }
    }

    /// 处理 Provider section 键盘事件
    /// 返回 true 表示事件被消费，false 表示未处理
    pub fn handle_provider_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        if self.provider_editor.is_some() {
            return self.handle_provider_editor_key(key);
        }
        if let Some(popup_idx) = self.provider_popup {
            match key {
                KeyCode::Esc => {
                    self.provider_popup = None;
                    true
                }
                KeyCode::Enter => {
                    if let Some(p) = self.providers.get(popup_idx) {
                        if let Some(first) = p.models.first() {
                            self.active_provider_idx = Some(popup_idx);
                            self.current_model = first.id.clone();
                            self.model_just_switched = true;
                        }
                    }
                    self.provider_popup = None;
                    true
                }
                _ => false,
            }
        } else {
            match key {
                KeyCode::Up => {
                    if self.provider_cursor > 0 {
                        self.provider_cursor -= 1;
                    }
                    true
                }
                KeyCode::Down => {
                    if self.provider_cursor < self.providers.len() + 1 {
                        self.provider_cursor += 1;
                    }
                    true
                }
                KeyCode::Enter => {
                    if self.provider_cursor == 0 {
                        // title 行，暂无可触发操作
                        return true;
                    }
                    if self.provider_cursor == self.providers.len() + 1 || self.providers.is_empty()
                    {
                        self.provider_popup = None;
                        self.provider_editor = Some(ProviderEditor {
                            is_new: true,
                            index: self.providers.len(),
                            draft: crate::provider::ProviderInfo {
                                id: String::new(),
                                name: String::new(),
                                enabled: true,
                                base_url: String::new(),
                                api_key: String::new(),
                                models: vec![],
                                endpoint_type: "openai_compat".into(),
                            },
                            field_focus: 6,
                            models_text: String::new(),
                            models_fetching: false,
                            model_mgr: None,
                        });
                    } else if !self.providers.is_empty() {
                        self.open_edit_provider_editor(self.provider_cursor - 1);
                    }
                    true
                }
                KeyCode::Char(' ')
                    if !self.providers.is_empty()
                        && self.provider_cursor > 0
                        && self.provider_cursor <= self.providers.len() =>
                {
                    // Space → toggle：激活 / 取消激活（cursor 偏移 1 = 列表第一项）
                    let p_idx = self.provider_cursor - 1;
                    if let Some(p) = self.providers.get(p_idx) {
                        let already_active = self.active_provider_idx == Some(p_idx);
                        if already_active {
                            self.active_provider_idx = None;
                            self.current_model.clear();
                        } else if let Some(first) = p.models.first() {
                            self.active_provider_idx = Some(p_idx);
                            self.current_model = first.id.clone();
                        }
                        self.model_just_switched = true;
                    }
                    true
                }
                KeyCode::Char('e')
                    if !self.providers.is_empty()
                        && self.provider_cursor > 0
                        && self.provider_cursor <= self.providers.len() =>
                {
                    // e → 编辑 provider（cursor 偏移 1 = 列表第一项）
                    self.open_edit_provider_editor(self.provider_cursor - 1);
                    true
                }
                KeyCode::Char('d')
                    if !self.providers.is_empty()
                        && self.provider_cursor < self.providers.len() =>
                {
                    // d → 切换 provider enabled/disabled
                    if let Some(p) = self.providers.get_mut(self.provider_cursor) {
                        p.enabled = !p.enabled;
                        // 禁用活跃 Provider 时清除选中状态
                        if !p.enabled && self.active_provider_idx == Some(self.provider_cursor) {
                            self.active_provider_idx = None;
                            self.current_model.clear();
                        }
                        self.sync_provider_config();
                    }
                    true
                }
                KeyCode::Left | KeyCode::Esc => true,
                KeyCode::Char('+') => {
                    let default = crate::provider::ProviderInfo {
                        id: "new-provider".into(),
                        name: "New Provider".into(),
                        enabled: true,
                        base_url: "https://api.openai.com/v1".into(),
                        api_key: String::new(),
                        endpoint_type: "openai_compat".into(),
                        models: vec![crate::provider::ModelInfo {
                            thinking_level_map: None,
                            id: "gpt-4o".into(),
                            name: "GPT-4o".into(),
                            context_window: 128000,
                            reasoning: true,
                            tier: "T3".into(),
                            enabled: true,
                        }],
                    };
                    self.providers.push(default);
                    self.sync_provider_config();
                    true
                }
                _ => false,
            }
        }
    }

    /// 统一保存 provider 配置到磁盘，并同步更新与 router 共享的内存配置
    pub fn sync_provider_config(&self) {
        // 持久化是否被禁用（单元测试关闭以避免污染真实配置）
        if self.persistence_disabled {
            return;
        }
        let path = crate::provider::config_path();
        let cfg = crate::provider::ProviderConfig {
            port: self.router_port,
            current_model: Some(self.current_model.clone()),
            current_provider: self.active_provider_idx,
            providers: self.providers.clone(),
        };
        crate::provider::ProviderConfig::save(&path, &cfg);
        // 重新生成 local-provider.ts，确保 pi 下次启动时使用正确的端点类型
        crate::provider::regenerate_local_provider_ts(&cfg);
        // 同步到 router 共享的内存配置（若不可用则仅写磁盘）
        if let Some(ref shared) = self.shared_config {
            match shared.try_write() {
                Ok(mut guard) => {
                    *guard = cfg;
                }
                Err(_) => {
                    tracing::warn!("无法获取 SharedConfig 写锁，router 可能使用过期配置");
                }
            }
        }
    }

    /// 处理 MODEL 子区键盘事件（支持搜索过滤 + 滚动）
    pub fn handle_model_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        if self.provider_editor.is_some() {
            return false;
        }
        match key {
            KeyCode::Up => {
                if self.model_cursor > 0 {
                    self.model_cursor -= 1;
                }
                true
            }
            KeyCode::Down => {
                let filtered = self.filtered_models();
                if self.model_cursor < filtered.len() {
                    self.model_cursor += 1;
                }
                true
            }
            KeyCode::Enter => {
                if self.model_cursor == 0 {
                    return true; // MODEL title，暂无可触发操作
                }
                let model_id = {
                    let filtered = self.filtered_models();
                    filtered.get(self.model_cursor - 1).map(|m| m.id.clone())
                };
                if let Some(mid) = model_id {
                    // MODEL 区显示的模型均来自当前活跃 Provider，直接用其索引
                    self.active_provider_idx = self.active_provider_index();
                    self.current_model = mid;
                    self.model_just_switched = true;
                }
                true
            }
            KeyCode::Char(' ') => {
                if self.model_cursor == 0 {
                    return true; // MODEL title，暂无可触发操作
                }
                // Space → toggle：切换模型选中/取消
                let model_id = {
                    let filtered = self.filtered_models();
                    filtered.get(self.model_cursor - 1).map(|m| m.id.clone())
                };
                if let Some(ref mid) = model_id {
                    if self.current_model == *mid {
                        // 当前选中 → 取消（保留 active_provider_idx，MODEL 区仍显示当前 Provider 模型）
                        self.current_model.clear();
                    } else {
                        // 未选中 → 选中（MODEL 区模型来自当前活跃 Provider）
                        self.active_provider_idx = self.active_provider_index();
                        self.current_model = mid.clone();
                    }
                    self.model_just_switched = true;
                }
                true
            }
            KeyCode::Backspace => {
                self.model_search.pop();
                self.model_cursor = 0;
                true
            }
            KeyCode::Esc => {
                self.model_search.clear();
                self.model_cursor = 0;
                true
            }
            KeyCode::Char(c) if c != ' ' => {
                self.model_search.push(c);
                self.model_cursor = 0;
                true
            }
            _ => false,
        }
    }

    /// 返回已开启 + 搜索匹配的模型列表
    pub fn filtered_models(&self) -> Vec<&crate::provider::ModelInfo> {
        let ap = match self.active_provider_for_models() {
            Some(p) => p,
            None => return vec![],
        };
        let q = self.model_search.to_lowercase();
        ap.models
            .iter()
            .filter(|m| {
                m.enabled && (self.model_search.is_empty() || m.id.to_lowercase().contains(&q))
            })
            .collect()
    }

    /// 获取当前活跃 provider（用于 MODEL 子区显示模型列表）
    /// 优先按 active_provider_idx 查找，其次按 current_model 匹配，只考虑启用的 provider
    pub fn active_provider_for_models(&self) -> Option<&crate::provider::ProviderInfo> {
        self.active_provider_index()
            .and_then(|idx| self.providers.get(idx))
    }

    /// 获取当前活跃 provider 的索引
    /// 优先按 active_provider_idx → current_model 匹配 → 第一个启用的 Provider
    pub fn active_provider_index(&self) -> Option<usize> {
        // 优先使用按索引选中的 Provider
        if let Some(idx) = self.active_provider_idx {
            if let Some(p) = self.providers.get(idx) {
                if p.enabled {
                    return Some(idx);
                }
            }
        }
        // 回退：按 current_model 匹配
        if let Some((i, _)) = self
            .providers
            .iter()
            .enumerate()
            .filter(|(_, p)| p.enabled)
            .find(|(_, p)| p.models.iter().any(|m| m.id == self.current_model))
        {
            return Some(i);
        }
        // 最终回退：第一个启用的 Provider
        self.providers
            .iter()
            .enumerate()
            .find(|(_, p)| p.enabled)
            .map(|(i, _)| i)
    }

    fn handle_provider_editor_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        if self.provider_editor.as_ref().unwrap().model_mgr.is_some() {
            return self.handle_model_manager_key(key);
        }
        match key {
            KeyCode::Esc => {
                self.provider_editor = None;
                true
            }
            KeyCode::Tab => {
                if let Some(ref mut editor) = self.provider_editor {
                    let old = editor.field_focus;
                    let new = next_field(old);
                    editor.field_focus = new;
                    if old == 3
                        && new != 3
                        && !editor.draft.base_url.trim().is_empty()
                        && !editor.draft.api_key.trim().is_empty()
                        && !editor.models_fetching
                    {
                        editor.models_fetching = true;
                    }
                }
                true
            }
            KeyCode::Enter => {
                if self.provider_editor.as_ref().unwrap().field_focus == 5 {
                    let editor = self.provider_editor.as_mut().unwrap();
                    editor.draft.models =
                        merge_models_text(&editor.models_text, &editor.draft.models);
                    editor.model_mgr = Some(ModelManager {
                        cursor: 0,
                        editor: None,
                    });
                    return true;
                }
                let editor = self.provider_editor.take().unwrap();
                let idx = editor.index;
                let draft = editor.draft;
                if editor.is_new {
                    self.providers.push(draft);
                } else if idx < self.providers.len() {
                    self.providers[idx] = draft;
                }
                self.sync_provider_config();
                true
            }
            KeyCode::Backspace => {
                if let Some(ref mut editor) = self.provider_editor {
                    match editor.field_focus {
                        0 => {
                            editor.draft.id.pop();
                        }
                        1 => {
                            editor.draft.name.pop();
                        }
                        2 => {
                            editor.draft.base_url.pop();
                        }
                        3 => {
                            editor.draft.api_key.pop();
                        }
                        _ => {}
                    }
                }
                true
            }
            KeyCode::Char(' ') => {
                if let Some(ref mut editor) = self.provider_editor {
                    if editor.field_focus == 6 {
                        // 端点类型循环切换
                        editor.draft.endpoint_type = match editor.draft.endpoint_type.as_str() {
                            "openai_responses" => "anthropic_messages".into(),
                            "anthropic_messages" => "gemini".into(),
                            "gemini" => "openai_compat".into(),
                            _ => "openai_responses".into(),
                        };
                    } else if editor.field_focus != 5 {
                        match editor.field_focus {
                            0 => {
                                editor.draft.id.push(' ');
                            }
                            1 => {
                                editor.draft.name.push(' ');
                            }
                            2 => {
                                editor.draft.base_url.push(' ');
                            }
                            3 => {
                                editor.draft.api_key.push(' ');
                            }
                            _ => {}
                        }
                    }
                }
                true
            }
            KeyCode::Char(c) => {
                if let Some(ref mut editor) = self.provider_editor {
                    match editor.field_focus {
                        0 => {
                            editor.draft.id.push(c);
                        }
                        1 => {
                            editor.draft.name.push(c);
                        }
                        2 => {
                            editor.draft.base_url.push(c);
                        }
                        3 => {
                            editor.draft.api_key.push(c);
                        }
                        5..=6 => {}
                        _ => {}
                    }
                }
                true
            }
            KeyCode::Up => {
                if let Some(ref mut editor) = self.provider_editor {
                    let old = editor.field_focus;
                    let new = prev_field(old);
                    editor.field_focus = new;
                    if old == 3
                        && new != 3
                        && !editor.draft.base_url.trim().is_empty()
                        && !editor.draft.api_key.trim().is_empty()
                        && !editor.models_fetching
                    {
                        editor.models_fetching = true;
                    }
                }
                true
            }
            KeyCode::Down => {
                if let Some(ref mut editor) = self.provider_editor {
                    let old = editor.field_focus;
                    let new = next_field(old);
                    editor.field_focus = new;
                    if old == 3
                        && new != 3
                        && !editor.draft.base_url.trim().is_empty()
                        && !editor.draft.api_key.trim().is_empty()
                        && !editor.models_fetching
                    {
                        editor.models_fetching = true;
                    }
                }
                true
            }
            _ => false,
        }
    }

    fn handle_model_manager_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        if self
            .provider_editor
            .as_ref()
            .unwrap()
            .model_mgr
            .as_ref()
            .unwrap()
            .editor
            .is_some()
        {
            return self.handle_model_field_editor_key(key);
        }
        let editor = self.provider_editor.as_mut().unwrap();
        let mgr = editor.model_mgr.as_mut().unwrap();
        let mc = editor.draft.models.len();
        let last = mc + 1;
        match key {
            KeyCode::Up | KeyCode::Char('k') => {
                if mgr.cursor > 0 {
                    mgr.cursor -= 1;
                }
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if mgr.cursor < last {
                    mgr.cursor += 1;
                }
                true
            }
            KeyCode::Enter => {
                if mgr.cursor < mc {
                    let m = editor.draft.models[mgr.cursor].clone();
                    mgr.editor = Some(ModelFieldEditor {
                        field_focus: 0,
                        draft_id: m.id.clone(),
                        draft_tier: m.tier.clone(),
                        draft_ctx: m.context_window.to_string(),
                        model_index: mgr.cursor,
                    });
                } else if mgr.cursor == mc {
                    let idx = editor.draft.models.len();
                    editor.draft.models.push(crate::provider::ModelInfo {
                        thinking_level_map: None,
                        id: "new-model".into(),
                        name: String::new(),
                        context_window: 128000,
                        reasoning: true,
                        tier: "T2".into(),
                        enabled: true,
                    });
                    mgr.cursor = idx;
                    mgr.editor = Some(ModelFieldEditor {
                        field_focus: 0,
                        draft_id: "new-model".into(),
                        draft_tier: "T2".into(),
                        draft_ctx: "128000".into(),
                        model_index: idx,
                    });
                } else {
                    editor.models_text = rebuild_models_text(&editor.draft.models);
                    editor.model_mgr = None;
                    let taken = self.provider_editor.take().unwrap();
                    let idx = taken.index;
                    let draft = taken.draft;
                    if taken.is_new {
                        self.providers.push(draft);
                    } else if idx < self.providers.len() {
                        self.providers[idx] = draft;
                    }
                    self.sync_provider_config();
                }
                true
            }
            KeyCode::Delete | KeyCode::Backspace => {
                if mgr.cursor < mc {
                    editor.draft.models.remove(mgr.cursor);
                    if mgr.cursor > 0 && mgr.cursor >= editor.draft.models.len() {
                        mgr.cursor = editor.draft.models.len();
                    }
                }
                true
            }
            KeyCode::Char(' ') => {
                if mgr.cursor < mc {
                    if let Some(m) = editor.draft.models.get_mut(mgr.cursor) {
                        m.enabled = !m.enabled;
                    }
                }
                true
            }
            KeyCode::Tab => true,
            KeyCode::Esc => {
                editor.models_text = rebuild_models_text(&editor.draft.models);
                editor.model_mgr = None;
                true
            }
            _ => false,
        }
    }

    fn handle_model_field_editor_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        const FO: [usize; 3] = [0, 1, 2];
        let editor = self.provider_editor.as_mut().unwrap();
        let mgr = editor.model_mgr.as_mut().unwrap();
        let fe = mgr.editor.as_mut().unwrap();
        match key {
            KeyCode::Up | KeyCode::Down | KeyCode::Tab => {
                let pos = FO.iter().position(|&f| f == fe.field_focus).unwrap_or(0);
                fe.field_focus = FO[match key {
                    KeyCode::Up => (pos + 2) % 3,
                    _ => (pos + 1) % 3,
                }];
                true
            }
            KeyCode::Enter => {
                let tu = fe.draft_tier.to_uppercase();
                if fe.draft_id.trim().is_empty() {
                    fe.field_focus = 0;
                    return true;
                }
                if tu != "T1" && tu != "T2" && tu != "T3" {
                    fe.field_focus = 1;
                    return true;
                }
                match fe.draft_ctx.parse::<u32>() {
                    Ok(v) if v > 0 => {}
                    _ => {
                        fe.field_focus = 2;
                        return true;
                    }
                }
                if fe.model_index < editor.draft.models.len() {
                    editor.draft.models[fe.model_index] = crate::provider::ModelInfo {
                        thinking_level_map: None,
                        id: fe.draft_id.trim().to_string(),
                        name: String::new(),
                        context_window: fe.draft_ctx.parse().unwrap_or(128000),
                        reasoning: tu != "T1",
                        tier: tu,
                        enabled: true,
                    };
                }
                mgr.editor = None;
                true
            }
            KeyCode::Esc => {
                mgr.editor = None;
                true
            }
            KeyCode::Backspace => {
                match fe.field_focus {
                    0 => {
                        fe.draft_id.pop();
                    }
                    1 => {
                        fe.draft_tier.pop();
                    }
                    2 => {
                        fe.draft_ctx.pop();
                    }
                    _ => {}
                }
                true
            }
            KeyCode::Char(c) => {
                match fe.field_focus {
                    0 => {
                        fe.draft_id.push(c);
                    }
                    1 => {
                        fe.draft_tier.push(c);
                    }
                    2 => {
                        fe.draft_ctx.push(c);
                    }
                    _ => {}
                }
                true
            }
            _ => false,
        }
    }

    /// cursor 0 = WORKSPACE title（无操作），1..=N = 树节点
    pub fn sidebar_item_at(&self, cursor: usize) -> Option<(bool, usize, Option<usize>)> {
        if cursor == 0 {
            return None; // WORKSPACE title
        }
        let mut idx = 1;
        for (wi, ws) in self.workspaces.iter().enumerate() {
            if idx == cursor {
                return Some((true, wi, None));
            }
            idx += 1;
            if ws.expanded {
                for si in 0..ws.sessions.len() {
                    if idx == cursor {
                        return Some((false, wi, Some(si)));
                    }
                    idx += 1;
                }
            }
        }
        None
    }
}
