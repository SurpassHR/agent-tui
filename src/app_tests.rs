use crate::action::Action;
use crate::app::App;
use crate::message::ChatMessage;
use crate::state::*;
use crate::utils::compute_diff_lines;

/// 解析 agent .md 文件的 YAML frontmatter
#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use tokio::sync::mpsc;

    use super::*;

    fn row_text(terminal: &Terminal<TestBackend>, y: u16, width: u16) -> String {
        (0..width)
            .map(|x| terminal.backend().buffer()[(x, y)].symbol())
            .collect()
    }

    #[test]
    fn render_tui_should_use_three_column_layout_without_top_bar() {
        let (action_tx, _action_rx) = mpsc::channel::<Action>(8);
        let mut app = App::new_rpc(action_tx);
        app.agent_status = AgentStatus::Idle;
        app.session.name = Some("demo-session".to_string());
        app.runtime.model_name = Some("deepseek-v4-flash".to_string());
        app.runtime.provider = Some("openrouter".to_string());
        // 设置工作区树含活跃会话（供 ACTIVE SESSION 区渲染）
        app.active_sessions.insert("demo-session".into());
        app.tui.workspaces.push(WorkspaceNode {
            cwd: "/test".into(),
            display_name: "test".into(),
            sessions: vec![SessionNode {
                id: "demo-session".into(),
                name: "demo-session".into(),
                file_path: None,
                message_count: 0,
                is_online: true,
            }],
            expanded: false,
        });

        let backend = TestBackend::new(120, 32);
        let mut terminal = Terminal::new(backend).expect("创建测试终端失败");

        terminal
            .draw(|f| {
                app.render_tui(f);
            })
            .expect("渲染 TUI 失败");

        // 第 0 行：sidebar 以空格开头（非聚焦时 ACTIVE SESSION 前有空格）
        let buffer = terminal.backend().buffer();
        assert_eq!(
            buffer[(0, 0)].symbol(),
            " ",
            "row 0 should start with space before ACTIVE SESSION header"
        );

        // 第 1 行是活跃会话行（● demo-session），左侧有 1 列内边距
        assert_eq!(
            buffer[(1, 1)].symbol(),
            "●",
            "row 1 should start with active session dot (after left padding)"
        );

        // 验证三栏分隔符位置
        let separators: Vec<u16> = (0..120)
            .filter(|x| buffer[(*x, 0)].symbol() == "│")
            .collect();
        assert_eq!(separators, vec![39, 80], "separators at expected positions");

        assert_eq!(
            buffer[(81, 0)].symbol(),
            " ",
            "agent panel should have left padding space (col 81)"
        );

        // 验证底栏存在（TestBackend 创建为 120x32）
        let last_row = 31u16;
        let bottom = row_text(&terminal, last_row, 120);
        assert!(
            bottom.contains("Ctrl+C"),
            "bottom bar should show shortcuts"
        );
    }

    #[test]
    fn test_provider_enter_opens_popup() {
        // RED: handle_provider_key 还不存在，因此需要先写测试
        // 期望：在 Provider section 按下 Enter 时 provider_popup 变为 Some(0)
        let mut state = TuiState::new();
        state.providers = vec![crate::provider::ProviderInfo {
            endpoint_type: "openai_compat".into(),
            id: "deepseek".into(),
            name: "DeepSeek".into(),
            enabled: true,
            base_url: "https://api.deepseek.com/v1".into(),
            api_key: "sk-test".into(),
            models: vec![crate::provider::ModelInfo {
                thinking_level_map: None,
                id: "deepseek-chat".into(),
                name: "DeepSeek Chat".into(),
                context_window: 64000,
                reasoning: false,
                tier: "T2".into(),
                enabled: true,
            }],
        }];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Provider;
        state.provider_popup = None;
        state.provider_cursor = 1; // 第一个 provider（title=0）

        state.handle_provider_key(crossterm::event::KeyCode::Enter);

        let editor = state
            .provider_editor
            .as_ref()
            .expect("Enter 应该打开 Provider 编辑表单");
        assert!(!editor.is_new, "编辑已有 provider 时 is_new 应为 false");
        assert_eq!(
            editor.draft.id, "deepseek",
            "编辑表单应预填充 provider 数据"
        );
        assert_eq!(state.provider_popup, None, "不应同时打开 popup");
    }

    #[test]
    fn test_provider_enter_on_empty_list_keeps_popup_closed() {
        let mut state = TuiState::new();
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Provider;
        state.provider_popup = None;
        state.provider_cursor = 0;

        state.handle_provider_key(crossterm::event::KeyCode::Enter);

        assert_eq!(
            state.provider_popup, None,
            "无 provider 时 Enter 不应打开 popup"
        );
    }

    #[test]
    fn test_provider_popup_esc_closes() {
        let mut state = TuiState::new();
        state.providers = vec![crate::provider::ProviderInfo {
            endpoint_type: "openai_compat".into(),
            id: "deepseek".into(),
            name: "DeepSeek".into(),
            enabled: true,
            base_url: "https://api.deepseek.com/v1".into(),
            api_key: "sk-test".into(),
            models: vec![],
        }];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Provider;
        state.provider_popup = Some(0);

        // Esc 关闭 popup
        state.handle_provider_key(crossterm::event::KeyCode::Esc);
        assert_eq!(state.provider_popup, None, "Esc 应关闭 popup");
    }

    #[test]
    fn test_provider_popup_enter_activates_first_model() {
        let mut state = TuiState::new();
        state.current_model = "old-model".into();
        state.providers = vec![crate::provider::ProviderInfo {
            endpoint_type: "openai_compat".into(),
            id: "deepseek".into(),
            name: "DeepSeek".into(),
            enabled: true,
            base_url: "https://api.deepseek.com/v1".into(),
            api_key: "sk-test".into(),
            models: vec![crate::provider::ModelInfo {
                thinking_level_map: None,
                id: "deepseek-chat".into(),
                name: "DeepSeek Chat".into(),
                context_window: 64000,
                reasoning: false,
                tier: "T2".into(),
                enabled: true,
            }],
        }];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Provider;
        state.provider_popup = Some(0);

        // Enter 激活第一个 model
        state.handle_provider_key(crossterm::event::KeyCode::Enter);
        assert_eq!(
            state.current_model, "deepseek-chat",
            "Enter 应激活第一个 model"
        );
        assert_eq!(
            state.active_provider_idx,
            Some(0),
            "Enter 应设置 active_provider_idx"
        );
        assert_eq!(state.provider_popup, None, "激活后 popup 应关闭");
    }

    #[test]
    fn test_provider_nav_down_up() {
        let mut state = TuiState::new();
        state.providers = vec![
            crate::provider::ProviderInfo {
                endpoint_type: "openai_compat".into(),
                id: "a".into(),
                name: "A".into(),
                enabled: true,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![],
            },
            crate::provider::ProviderInfo {
                endpoint_type: "openai_compat".into(),
                id: "b".into(),
                name: "B".into(),
                enabled: true,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![],
            },
            crate::provider::ProviderInfo {
                endpoint_type: "openai_compat".into(),
                id: "c".into(),
                name: "C".into(),
                enabled: true,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![],
            },
        ];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Provider;
        state.provider_cursor = 0;

        // Down → cursor=1（第一个 provider）
        state.handle_provider_key(crossterm::event::KeyCode::Down);
        assert_eq!(state.provider_cursor, 1, "Down 应移动到第一个 provider");

        // Down → cursor=2（第二个 provider）
        state.handle_provider_key(crossterm::event::KeyCode::Down);
        assert_eq!(state.provider_cursor, 2, "Down 应移动到第二个 provider");

        // Down → cursor=3（第三个 provider）
        state.handle_provider_key(crossterm::event::KeyCode::Down);
        assert_eq!(state.provider_cursor, 3, "Down 应移动到第三个 provider");

        // Down → cursor=4（add provider row）
        state.handle_provider_key(crossterm::event::KeyCode::Down);
        assert_eq!(state.provider_cursor, 4, "Down 应到 add provider 行");

        // Down → stays at 4（end of list）
        state.handle_provider_key(crossterm::event::KeyCode::Down);
        assert_eq!(state.provider_cursor, 4, "Down 在末尾不应越界");

        // Up → cursor=3
        state.handle_provider_key(crossterm::event::KeyCode::Up);
        assert_eq!(state.provider_cursor, 3, "Up 应回到第三个");

        // Up → cursor=2
        state.handle_provider_key(crossterm::event::KeyCode::Up);
        assert_eq!(state.provider_cursor, 2, "Up 应回到第二个");

        // Up → cursor=1
        state.handle_provider_key(crossterm::event::KeyCode::Up);
        assert_eq!(state.provider_cursor, 1, "Up 应回到第一个");

        // Up → cursor=0（title）
        state.handle_provider_key(crossterm::event::KeyCode::Up);
        assert_eq!(state.provider_cursor, 0, "Up 应回到 title");

        // Up → stays at 0（start of list）
        state.handle_provider_key(crossterm::event::KeyCode::Up);
        assert_eq!(state.provider_cursor, 0, "Up 在开头不应越界");
    }

    #[test]
    fn test_model_nav_down_up_enter() {
        let mut state = TuiState::new();
        state.current_model = "m1".to_string(); // 预设当前模型
        state.active_provider_idx = Some(0); // 预设活跃 Provider
        state.providers = vec![crate::provider::ProviderInfo {
            endpoint_type: "openai_compat".into(),
            id: "ds".into(),
            name: "DS".into(),
            enabled: true,
            base_url: "".into(),
            api_key: "".into(),
            models: vec![
                crate::provider::ModelInfo {
                    thinking_level_map: None,
                    id: "m1".into(),
                    name: "M1".into(),
                    context_window: 1000,
                    reasoning: false,
                    tier: "T1".into(),
                    enabled: true,
                },
                crate::provider::ModelInfo {
                    thinking_level_map: None,
                    id: "m2".into(),
                    name: "M2".into(),
                    context_window: 2000,
                    reasoning: true,
                    tier: "T2".into(),
                    enabled: true,
                },
            ],
        }];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Model;
        state.model_cursor = 0;

        // Down → 第一个 model
        state.handle_model_key(crossterm::event::KeyCode::Down);
        assert_eq!(state.model_cursor, 1, "Down 应移动到第一个 model");

        // Down → 选择第二个 model
        state.handle_model_key(crossterm::event::KeyCode::Down);
        assert_eq!(state.model_cursor, 2, "Down 应移动到第二个 model");

        // Enter 选中 model
        state.handle_model_key(crossterm::event::KeyCode::Enter);
        assert_eq!(state.current_model, "m2", "Enter 应切换到选中的 model");
        assert_eq!(
            state.active_provider_idx,
            Some(0),
            "Enter 应保持当前活跃 Provider 不变"
        );
    }

    #[test]
    fn test_model_space_toggle_activate() {
        // Space 选中当前未选中的模型
        let mut state = TuiState::new();
        state.current_model = "m1".into();
        state.active_provider_idx = Some(0);
        state.providers = vec![crate::provider::ProviderInfo {
            endpoint_type: "openai_compat".into(),
            id: "ds".into(),
            name: "DS".into(),
            enabled: true,
            base_url: "".into(),
            api_key: "".into(),
            models: vec![
                crate::provider::ModelInfo {
                    thinking_level_map: None,
                    id: "m1".into(),
                    name: "M1".into(),
                    context_window: 1000,
                    reasoning: false,
                    tier: "T1".into(),
                    enabled: true,
                },
                crate::provider::ModelInfo {
                    thinking_level_map: None,
                    id: "m2".into(),
                    name: "M2".into(),
                    context_window: 2000,
                    reasoning: true,
                    tier: "T2".into(),
                    enabled: true,
                },
            ],
        }];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Model;
        state.model_cursor = 2; // 光标在 m2（未选中，title=0, m1=1, m2=2）

        state.handle_model_key(crossterm::event::KeyCode::Char(' '));
        assert_eq!(state.current_model, "m2", "Space 应切换到 m2");
        assert_eq!(state.active_provider_idx, Some(0), "不应切换 Provider");
        assert!(state.model_just_switched);
    }

    #[test]
    fn test_model_space_toggle_deactivate() {
        // Space 取消当前已选中的模型
        let mut state = TuiState::new();
        state.current_model = "m1".into();
        state.active_provider_idx = Some(0);
        state.providers = vec![crate::provider::ProviderInfo {
            endpoint_type: "openai_compat".into(),
            id: "ds".into(),
            name: "DS".into(),
            enabled: true,
            base_url: "".into(),
            api_key: "".into(),
            models: vec![crate::provider::ModelInfo {
                thinking_level_map: None,
                id: "m1".into(),
                name: "M1".into(),
                context_window: 1000,
                reasoning: false,
                tier: "T1".into(),
                enabled: true,
            }],
        }];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Model;
        state.model_cursor = 1; // 光标在 m1（已选中，title=0, m1=1）

        state.handle_model_key(crossterm::event::KeyCode::Char(' '));
        assert_eq!(state.current_model, "", "Space 应取消选中");
        assert_eq!(
            state.active_provider_idx,
            Some(0),
            "取消选中应保留 active_provider_idx（MODEL 区仍显示原 Provider 模型）"
        );
    }

    #[test]
    fn test_model_space_does_not_switch_provider_on_same_model_id() {
        // 两个 Provider 有同名模型，在 Provider 1 的 MODEL 区 Space 切换不应跳到 Provider 0
        let mut state = TuiState::new();
        state.providers = vec![
            crate::provider::ProviderInfo {
                endpoint_type: "openai_compat".into(),
                id: "elysiver".into(),
                name: "Elysiver".into(),
                enabled: true,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    thinking_level_map: None,
                    id: "shared-model".into(),
                    name: "Shared".into(),
                    context_window: 1000,
                    reasoning: false,
                    tier: "T2".into(),
                    enabled: true,
                }],
            },
            crate::provider::ProviderInfo {
                endpoint_type: "openai_compat".into(),
                id: "deepseek".into(),
                name: "DeepSeek".into(),
                enabled: true,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    thinking_level_map: None,
                    id: "shared-model".into(),
                    name: "Shared".into(),
                    context_window: 2000,
                    reasoning: true,
                    tier: "T2".into(),
                    enabled: true,
                }],
            },
        ];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Model;
        state.model_cursor = 1; // 第一个 model（title=0）
                                // 当前在 deepseek Provider (索引 1) 的 MODEL 区
        state.active_provider_idx = Some(1);
        state.current_model = "shared-model".into();

        // Space 取消选中
        state.handle_model_key(crossterm::event::KeyCode::Char(' '));
        assert_eq!(state.current_model, "", "Space 应取消选中");

        // Space 重新选中 — 应保持在 deepseek (索引 1)
        state.handle_model_key(crossterm::event::KeyCode::Char(' '));
        assert_eq!(state.current_model, "shared-model");
        assert_eq!(
            state.active_provider_idx,
            Some(1),
            "同名模型不应导致 Provider 跳到第一个 (elysiver)"
        );
    }

    #[test]
    fn test_model_enter_does_not_switch_provider_on_same_model_id() {
        // Enter 在 MODEL 区不应切换 Provider
        let mut state = TuiState::new();
        state.providers = vec![
            crate::provider::ProviderInfo {
                endpoint_type: "openai_compat".into(),
                id: "a".into(),
                name: "A".into(),
                enabled: true,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    thinking_level_map: None,
                    id: "dup".into(),
                    name: "Dup".into(),
                    context_window: 1000,
                    reasoning: false,
                    tier: "T1".into(),
                    enabled: true,
                }],
            },
            crate::provider::ProviderInfo {
                endpoint_type: "openai_compat".into(),
                id: "b".into(),
                name: "B".into(),
                enabled: true,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    thinking_level_map: None,
                    id: "dup".into(),
                    name: "Dup".into(),
                    context_window: 2000,
                    reasoning: true,
                    tier: "T2".into(),
                    enabled: true,
                }],
            },
        ];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Model;
        state.model_cursor = 1; // 第一个 model（title=0）
        state.active_provider_idx = Some(1); // 在 Provider B
        state.current_model = "other".into();

        state.handle_model_key(crossterm::event::KeyCode::Enter);
        assert_eq!(state.current_model, "dup");
        assert_eq!(
            state.active_provider_idx,
            Some(1),
            "Enter 不应把 Provider 从 B 切到 A"
        );
    }

    #[test]
    fn test_provider_space_sets_active_provider_idx() {
        let mut state = TuiState::new();
        state.providers = vec![crate::provider::ProviderInfo {
            endpoint_type: "openai_compat".into(),
            id: "ds".into(),
            name: "DS".into(),
            enabled: true,
            base_url: "".into(),
            api_key: "".into(),
            models: vec![crate::provider::ModelInfo {
                thinking_level_map: None,
                id: "m1".into(),
                name: "M1".into(),
                context_window: 1000,
                reasoning: false,
                tier: "T1".into(),
                enabled: true,
            }],
        }];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Provider;
        state.provider_cursor = 1; // 第一个 provider（title=0）

        state.handle_provider_key(crossterm::event::KeyCode::Char(' '));
        assert_eq!(state.active_provider_idx, Some(0));
        assert_eq!(state.current_model, "m1");
    }

    #[test]
    fn test_provider_space_toggle_deactivates() {
        let mut state = TuiState::new();
        state.providers = vec![crate::provider::ProviderInfo {
            endpoint_type: "openai_compat".into(),
            id: "ds".into(),
            name: "DS".into(),
            enabled: true,
            base_url: "".into(),
            api_key: "".into(),
            models: vec![crate::provider::ModelInfo {
                thinking_level_map: None,
                id: "m1".into(),
                name: "M1".into(),
                context_window: 1000,
                reasoning: false,
                tier: "T1".into(),
                enabled: true,
            }],
        }];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Provider;
        state.provider_cursor = 1; // 第一个 provider（title=0）
        state.active_provider_idx = Some(0);
        state.current_model = "m1".into();

        state.handle_provider_key(crossterm::event::KeyCode::Char(' '));
        assert_eq!(state.active_provider_idx, None);
        assert_eq!(state.current_model, "");
    }

    #[test]
    fn test_provider_space_does_not_cross_affect() {
        // 两个 Provider，Space 切换 A 不应影响 B 的活跃状态判断
        let mut state = TuiState::new();
        state.providers = vec![
            crate::provider::ProviderInfo {
                endpoint_type: "openai_compat".into(),
                id: "a".into(),
                name: "A".into(),
                enabled: true,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    thinking_level_map: None,
                    id: "ma".into(),
                    name: "MA".into(),
                    context_window: 1000,
                    reasoning: false,
                    tier: "T1".into(),
                    enabled: true,
                }],
            },
            crate::provider::ProviderInfo {
                endpoint_type: "openai_compat".into(),
                id: "b".into(),
                name: "B".into(),
                enabled: true,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    thinking_level_map: None,
                    id: "mb".into(),
                    name: "MB".into(),
                    context_window: 2000,
                    reasoning: true,
                    tier: "T2".into(),
                    enabled: true,
                }],
            },
        ];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Provider;

        // Space 切换 Provider A
        state.provider_cursor = 1; // 第一个 provider（title=0）
        state.handle_provider_key(crossterm::event::KeyCode::Char(' '));
        assert_eq!(state.active_provider_idx, Some(0));
        assert_eq!(state.current_model, "ma");

        // 切换到 Provider B
        state.provider_cursor = 2; // 第二个 provider
        state.handle_provider_key(crossterm::event::KeyCode::Char(' '));
        assert_eq!(
            state.active_provider_idx,
            Some(1),
            "Space 应激活 Provider B，而不是保持 A"
        );
        assert_eq!(state.current_model, "mb");

        // Provider A 已不再是活跃
        assert_ne!(state.active_provider_idx, Some(0));
    }

    #[test]
    fn test_provider_d_key_clears_active_idx_when_disabling_active() {
        let mut state = TuiState::new();
        state.persistence_disabled = true;
        state.providers = vec![crate::provider::ProviderInfo {
            endpoint_type: "openai_compat".into(),
            id: "ds".into(),
            name: "DS".into(),
            enabled: true,
            base_url: "".into(),
            api_key: "".into(),
            models: vec![crate::provider::ModelInfo {
                thinking_level_map: None,
                id: "m1".into(),
                name: "M1".into(),
                context_window: 1000,
                reasoning: false,
                tier: "T1".into(),
                enabled: true,
            }],
        }];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Provider;
        state.provider_cursor = 0;
        state.active_provider_idx = Some(0);
        state.current_model = "m1".into();

        // 禁用活跃 Provider
        state.handle_provider_key(crossterm::event::KeyCode::Char('d'));
        assert!(!state.providers[0].enabled, "Provider 应被禁用");
        assert_eq!(
            state.active_provider_idx, None,
            "禁用活跃 Provider 应清除 active_provider_idx"
        );
        assert_eq!(
            state.current_model, "",
            "禁用活跃 Provider 应清除 current_model"
        );
    }

    #[test]
    fn test_provider_d_key_keeps_active_idx_when_disabling_inactive() {
        // 禁用非活跃 Provider 不应清除 active_provider_idx
        let mut state = TuiState::new();
        state.persistence_disabled = true;
        state.providers = vec![
            crate::provider::ProviderInfo {
                endpoint_type: "openai_compat".into(),
                id: "a".into(),
                name: "A".into(),
                enabled: true,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    thinking_level_map: None,
                    id: "ma".into(),
                    name: "MA".into(),
                    context_window: 1000,
                    reasoning: false,
                    tier: "T1".into(),
                    enabled: true,
                }],
            },
            crate::provider::ProviderInfo {
                endpoint_type: "openai_compat".into(),
                id: "b".into(),
                name: "B".into(),
                enabled: true,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    thinking_level_map: None,
                    id: "mb".into(),
                    name: "MB".into(),
                    context_window: 2000,
                    reasoning: true,
                    tier: "T2".into(),
                    enabled: true,
                }],
            },
        ];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Provider;
        state.active_provider_idx = Some(0); // A 是活跃的
        state.current_model = "ma".into();

        // 禁用非活跃的 Provider B
        state.provider_cursor = 1;
        state.handle_provider_key(crossterm::event::KeyCode::Char('d'));
        assert!(!state.providers[1].enabled, "Provider B 应被禁用");
        assert_eq!(
            state.active_provider_idx,
            Some(0),
            "禁用非活跃 Provider 不应清除 active_provider_idx"
        );
        assert_eq!(state.current_model, "ma", "current_model 不应被清除");
    }

    #[test]
    fn test_active_provider_for_models_prefers_idx() {
        // active_provider_for_models 应优先使用 active_provider_idx
        let mut state = TuiState::new();
        state.current_model = "shared".into();
        state.providers = vec![
            crate::provider::ProviderInfo {
                endpoint_type: "openai_compat".into(),
                id: "first".into(),
                name: "First".into(),
                enabled: true,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    thinking_level_map: None,
                    id: "shared".into(),
                    name: "Shared".into(),
                    context_window: 1000,
                    reasoning: false,
                    tier: "T1".into(),
                    enabled: true,
                }],
            },
            crate::provider::ProviderInfo {
                endpoint_type: "openai_compat".into(),
                id: "second".into(),
                name: "Second".into(),
                enabled: true,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    thinking_level_map: None,
                    id: "shared".into(),
                    name: "Shared".into(),
                    context_window: 2000,
                    reasoning: true,
                    tier: "T2".into(),
                    enabled: true,
                }],
            },
        ];

        state.active_provider_idx = Some(1); // 指定 second
        let ap = state.active_provider_for_models().unwrap();
        assert_eq!(ap.id, "second", "应按 idx 返回 second，不是 first");
    }

    #[test]
    fn test_active_provider_for_models_fallback_to_current_model() {
        // 无 active_provider_idx 时回退到 current_model 匹配
        let mut state = TuiState::new();
        state.current_model = "model-b".into();
        state.providers = vec![
            crate::provider::ProviderInfo {
                endpoint_type: "openai_compat".into(),
                id: "a".into(),
                name: "A".into(),
                enabled: true,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    thinking_level_map: None,
                    id: "model-a".into(),
                    name: "MA".into(),
                    context_window: 1000,
                    reasoning: false,
                    tier: "T1".into(),
                    enabled: true,
                }],
            },
            crate::provider::ProviderInfo {
                endpoint_type: "openai_compat".into(),
                id: "b".into(),
                name: "B".into(),
                enabled: true,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    thinking_level_map: None,
                    id: "model-b".into(),
                    name: "MB".into(),
                    context_window: 2000,
                    reasoning: true,
                    tier: "T2".into(),
                    enabled: true,
                }],
            },
        ];

        state.active_provider_idx = None;
        let ap = state.active_provider_for_models().unwrap();
        assert_eq!(ap.id, "b", "回退应按 current_model 匹配找到 Provider B");
    }

    #[test]
    fn test_active_provider_index_prefers_idx() {
        let mut state = TuiState::new();
        state.current_model = "shared".into();
        state.providers = vec![
            crate::provider::ProviderInfo {
                endpoint_type: "openai_compat".into(),
                id: "first".into(),
                name: "First".into(),
                enabled: true,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    thinking_level_map: None,
                    id: "shared".into(),
                    name: "Shared".into(),
                    context_window: 1000,
                    reasoning: false,
                    tier: "T1".into(),
                    enabled: true,
                }],
            },
            crate::provider::ProviderInfo {
                endpoint_type: "openai_compat".into(),
                id: "second".into(),
                name: "Second".into(),
                enabled: true,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    thinking_level_map: None,
                    id: "shared".into(),
                    name: "Shared".into(),
                    context_window: 2000,
                    reasoning: true,
                    tier: "T2".into(),
                    enabled: true,
                }],
            },
        ];

        state.active_provider_idx = Some(1);
        let idx = state.active_provider_index().unwrap();
        assert_eq!(idx, 1, "应返回 idx=1，不按 current_model 匹配到 0");
    }

    #[test]
    fn test_active_provider_index_fallback() {
        let mut state = TuiState::new();
        state.current_model = "m2".into();
        state.providers = vec![crate::provider::ProviderInfo {
            endpoint_type: "openai_compat".into(),
            id: "p".into(),
            name: "P".into(),
            enabled: true,
            base_url: "".into(),
            api_key: "".into(),
            models: vec![
                crate::provider::ModelInfo {
                    thinking_level_map: None,
                    id: "m1".into(),
                    name: "M1".into(),
                    context_window: 1000,
                    reasoning: false,
                    tier: "T1".into(),
                    enabled: true,
                },
                crate::provider::ModelInfo {
                    thinking_level_map: None,
                    id: "m2".into(),
                    name: "M2".into(),
                    context_window: 2000,
                    reasoning: true,
                    tier: "T2".into(),
                    enabled: true,
                },
            ],
        }];

        state.active_provider_idx = None;
        let idx = state.active_provider_index().unwrap();
        assert_eq!(idx, 0, "回退应按 current_model='m2' 匹配到索引 0");
    }

    #[test]
    fn test_filtered_models_uses_active_provider() {
        // filtered_models 应从 active_provider_for_models 返回的 Provider 取模型
        let mut state = TuiState::new();
        state.current_model = "m1".into();
        state.providers = vec![
            crate::provider::ProviderInfo {
                endpoint_type: "openai_compat".into(),
                id: "p0".into(),
                name: "P0".into(),
                enabled: true,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    thinking_level_map: None,
                    id: "m1".into(),
                    name: "M1".into(),
                    context_window: 1000,
                    reasoning: false,
                    tier: "T1".into(),
                    enabled: true,
                }],
            },
            crate::provider::ProviderInfo {
                endpoint_type: "openai_compat".into(),
                id: "p1".into(),
                name: "P1".into(),
                enabled: true,
                base_url: "".into(),
                api_key: "".into(),
                models: vec![crate::provider::ModelInfo {
                    thinking_level_map: None,
                    id: "m2".into(),
                    name: "M2".into(),
                    context_window: 2000,
                    reasoning: true,
                    tier: "T2".into(),
                    enabled: true,
                }],
            },
        ];
        state.active_provider_idx = Some(1); // 指定 p1

        let models = state.filtered_models();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "m2", "应从 p1 取模型，不是 p0");
    }

    #[test]
    fn test_handle_provider_key_space_on_disabled_provider_noop_via_guard() {
        // guard 条件已防止禁用/空 provider 触发 Space，此测试验证 guard 生效
        // disabled provider 的 cursor 仍可定位，但 Space 的 guard 检查列表非空+游标在范围内
        let mut state = TuiState::new();
        state.providers = vec![crate::provider::ProviderInfo {
            endpoint_type: "openai_compat".into(),
            id: "ds".into(),
            name: "DS".into(),
            enabled: false,
            base_url: "".into(),
            api_key: "".into(),
            models: vec![],
        }];
        state.focus_panel = FocusPanel::Sidebar;
        state.sidebar_subsection = SidebarSubsection::Provider;
        state.provider_cursor = 0;
        state.current_model = "x".into();

        // Space guard: provider_cursor < providers.len() → true
        // 但 provider has no models, so "else if" branch doesn't fire
        // current_model should remain unchanged
        state.handle_provider_key(crossterm::event::KeyCode::Char(' '));
        assert_eq!(
            state.current_model, "x",
            "无模型的 provider 不应切换 current_model"
        );
        assert_eq!(
            state.active_provider_idx, None,
            "无模型的 provider 不应设为活跃"
        );
    }

    #[test]
    fn test_toggle_block() {
        let (action_tx, _action_rx) = mpsc::channel::<Action>(8);
        let mut app = App::new_rpc(action_tx);
        app.tui.main_view.block_states.insert(
            "msg-1:0".to_string(),
            crate::message::BlockExpanded::Collapsed,
        );
        // 模拟 ToggleBlock
        let key = "msg-1:0".to_string();
        let state = app.tui.main_view.block_states.get_mut(&key).unwrap();
        *state = crate::message::BlockExpanded::Expanded;
        assert_eq!(*state, crate::message::BlockExpanded::Expanded);
    }

    #[test]
    fn test_exit_block() {
        let (action_tx, _action_rx) = mpsc::channel::<Action>(8);
        let mut app = App::new_rpc(action_tx);
        app.tui.main_view.entered_view = Some(crate::message::EnteredView::FullOutput {
            title: "test".into(),
            content: "hello".into(),
            scroll: 0,
        });
        app.tui.main_view.entered_view = None;
        assert!(app.tui.main_view.entered_view.is_none());
    }

    #[test]
    fn test_compute_diff_lines() {
        let old = "line1\nline2\nline3\n";
        let new = "line1\nline2_changed\nline3\nline4\n";
        let lines = compute_diff_lines(old, new);
        assert!(
            lines.iter().any(|l| l.kind == '-'),
            "should have deleted lines"
        );
        assert!(
            lines.iter().any(|l| l.kind == '+'),
            "should have added lines"
        );
        assert!(
            lines.iter().any(|l| l.kind == ' '),
            "should have unchanged lines"
        );
    }

    #[test]
    fn test_build_block_refs_empty() {
        let msgs: Vec<ChatMessage> = vec![];
        let refs = crate::message::build_block_refs(&msgs);
        assert!(refs.is_empty(), "empty messages should give no block refs");
    }

    #[test]
    fn test_build_block_refs_with_content() {
        let mut msg = ChatMessage::assistant("test", "");
        msg.content = vec![
            crate::message::ContentBlock::Thinking {
                thinking: "思考中...".to_string(),
            },
            crate::message::ContentBlock::Text {
                text: "回复内容".to_string(),
            },
            crate::message::ContentBlock::ToolCall {
                id: "tc-1".to_string(),
                name: "read".to_string(),
                arguments: serde_json::json!({"path": "src/main.rs"}),
                result: None,
                is_error: false,
            },
        ];
        let msgs = vec![msg];
        let refs = crate::message::build_block_refs(&msgs);
        assert_eq!(
            refs.len(),
            2,
            "Thinking + ToolCall = 2 blocks, Text skipped"
        );
        assert_eq!(refs[0].kind, crate::message::BlockKind::Thinking);
        assert_eq!(refs[1].kind, crate::message::BlockKind::ToolCall);
    }

    #[test]
    fn test_tool_event_running_creates_message() {
        let (action_tx, _action_rx) = mpsc::channel::<Action>(8);
        let mut app = App::new_rpc(action_tx);
        app.active_agent = Some("agent-1".to_string());

        // 发送 Running 事件
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            app.handle_action(Action::ToolEvent {
                agent_id: "agent-1".to_string(),
                tool_name: "bash".to_string(),
                tool_call_id: "tc-1".to_string(),
                status: crate::message::ToolStatus::Running,
                args: Some(serde_json::json!({"command": "ls"})),
                result: None,
                is_error: false,
            })
            .await
            .ok();
        });

        let msgs = app.messages.get("agent-1").unwrap();
        assert_eq!(msgs.len(), 1, "should have 1 tool message");
        assert_eq!(msgs[0].role, crate::message::ChatRole::Tool);
    }

    #[test]
    fn test_tool_event_done_updates_existing() {
        let (action_tx, _action_rx) = mpsc::channel::<Action>(8);
        let mut app = App::new_rpc(action_tx);
        app.active_agent = Some("agent-1".to_string());

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // 先发送 Running
            app.handle_action(Action::ToolEvent {
                agent_id: "agent-1".to_string(),
                tool_name: "bash".to_string(),
                tool_call_id: "tc-1".to_string(),
                status: crate::message::ToolStatus::Running,
                args: Some(serde_json::json!({"command": "ls"})),
                result: None,
                is_error: false,
            })
            .await
            .ok();

            // 再发送 Done（同一 tool_call_id）
            app.handle_action(Action::ToolEvent {
                agent_id: "agent-1".to_string(),
                tool_name: "bash".to_string(),
                tool_call_id: "tc-1".to_string(),
                status: crate::message::ToolStatus::Done,
                args: None,
                result: Some(serde_json::json!({"content": "ok"})),
                is_error: false,
            })
            .await
            .ok();
        });

        let msgs = app.messages.get("agent-1").unwrap();
        assert_eq!(
            msgs.len(),
            1,
            "should still be 1 tool message (updated, not duplicated)"
        );
        assert_eq!(
            msgs[0].tool_call.as_ref().unwrap().status,
            crate::message::ToolStatus::Done
        );
        assert!(msgs[0].text.contains("✓"), "Done should show ✓");
    }

    #[test]
    fn test_is_online_from_active_sessions() {
        let mut app = App::new_rpc(tokio::sync::mpsc::channel::<Action>(1).0);
        app.tui.workspaces.push(WorkspaceNode {
            cwd: "/test".into(),
            display_name: "test".into(),
            sessions: vec![SessionNode {
                id: "sess-1".into(),
                name: "Test".into(),
                file_path: None,
                message_count: 0,
                is_online: false,
            }],
            expanded: true,
        });

        // 未连接时 is_online 为 false
        app.sync_components();
        assert!(!app.tui.workspaces[0].sessions[0].is_online);

        // 连接后 is_online 为 true
        app.active_sessions.insert("sess-1".into());
        app.sync_components();
        assert!(app.tui.workspaces[0].sessions[0].is_online);

        // 断开后 is_online 恢复为 false
        app.active_sessions.remove("sess-1");
        app.sync_components();
        assert!(!app.tui.workspaces[0].sessions[0].is_online);
    }

    #[test]
    fn test_persist_includes_active_agent_sessions() {
        let mut app = App::new_rpc(tokio::sync::mpsc::channel::<Action>(1).0);
        app.active_sessions.insert("sess-a".into());
        app.active_sessions.insert("sess-b".into());

        let state = app.build_persist_state();
        assert_eq!(state.active_agent_sessions.len(), 2);
        assert!(state.active_agent_sessions.contains(&"sess-a".to_string()));
        assert!(state.active_agent_sessions.contains(&"sess-b".to_string()));
    }

    /// 测试：active_agent 为非 "default" 时，用户消息和 AI 回复写入同一个条目
    ///
    /// 这是回归测试：修复前，默认 pi 的事件转发硬编码 "default"，
    /// 导致 AI 回复写入 self.messages["default"]，而用户消息写入 self.messages[session_id]，
    /// sync_components 只同步 active_agent 对应的条目，AI 回复永远不可见。
    #[tokio::test]
    async fn test_active_agent_consistency_user_and_assistant_same_entry() {
        let mut app = App::new_rpc(tokio::sync::mpsc::channel::<Action>(1).0);
        // 模拟会话恢复后的状态：active_agent 不是 "default"
        let session_id = "2026-06-25T03-33-session".to_string();
        app.active_agent = Some(session_id.clone());

        // 1. 用户输入
        app.handle_action(Action::UserSubmitInput("你好".to_string()))
            .await
            .unwrap();

        // 2. AI 回复事件（agent_id 应与 active_agent 一致）
        app.handle_action(Action::ContentUpdate {
            agent_id: session_id.clone(),
            content: vec![crate::message::ContentBlock::Text {
                text: "你好！有什么可以帮助你的？".to_string(),
            }],
        })
        .await
        .unwrap();
        app.handle_action(Action::MessageAppend {
            agent_id: session_id.clone(),
            text: "你好！有什么可以帮助你的？".to_string(),
        })
        .await
        .unwrap();

        // 验证：用户和 AI 消息在同一个条目中
        let msgs = app.messages.get(&session_id).unwrap();
        assert_eq!(msgs.len(), 2, "应该包含用户消息和 AI 回复");
        assert!(matches!(msgs[0].role, crate::message::ChatRole::User));
        assert!(matches!(msgs[1].role, crate::message::ChatRole::Assistant));

        // 验证："default" 条目中没有消息（不应该泄漏）
        assert!(
            app.messages.get("default").map_or(true, |m| m.is_empty()),
            "'default' 条目不应有消息"
        );
    }

    /// 测试：sync_components 通过 watch channel 更新 forwarding_agent_tx
    ///
    /// 验证当 active_agent 变化时，forwarding_agent_tx 会收到新值。
    #[tokio::test]
    async fn test_forwarding_agent_tx_updates_on_active_agent_change() {
        let mut app = App::new_rpc(tokio::sync::mpsc::channel::<Action>(1).0);
        let initial_id = "default".to_string();

        // 创建 watch channel（模拟 run_tui 中的设置）
        let (forwarding_tx, forwarding_rx) = tokio::sync::watch::channel(initial_id.clone());
        app.forwarding_agent_tx = Some(forwarding_tx);

        assert_eq!(*forwarding_rx.borrow(), initial_id);

        // 模拟会话恢复：active_agent 变为 session_id
        let session_id = "restored-session-123".to_string();
        app.active_agent = Some(session_id.clone());

        // sync_components 应该将新 agent_id 写入 watch channel
        app.sync_components();

        // forwarding_rx 应该收到更新
        assert_eq!(*forwarding_rx.borrow(), session_id);
    }

    /// 测试：会话恢复后主视图包含正确消息
    ///
    /// 模拟完整流程：
    /// session restore → 用户输入 → AI 回复 → sync_components → main_view 包含所有消息
    #[tokio::test]
    async fn test_main_view_has_both_messages_after_session_restore() {
        let mut app = App::new_rpc(tokio::sync::mpsc::channel::<Action>(1).0);
        let session_id = "restored-session-abc".to_string();

        // 模拟 run_tui 中的初始化：设置 forwarding_agent_tx
        let (forwarding_tx, _forwarding_rx) = tokio::sync::watch::channel(session_id.clone());
        app.forwarding_agent_tx = Some(forwarding_tx);
        app.active_agent = Some(session_id.clone());

        // 1. 用户发送消息
        app.handle_action(Action::UserSubmitInput("hello".to_string()))
            .await
            .unwrap();

        // 2. sync_components 更新 main_view（render_tui 每帧调用）
        app.sync_components();
        assert_eq!(app.tui.main_view.messages.len(), 1);
        assert!(matches!(
            app.tui.main_view.messages[0].role,
            crate::message::ChatRole::User
        ));

        // 3. AI 回复（agent_id 来自 forwarding_agent_tx = session_id）
        app.handle_action(Action::ContentUpdate {
            agent_id: session_id.clone(),
            content: vec![crate::message::ContentBlock::Text {
                text: "Hello! How can I help?".to_string(),
            }],
        })
        .await
        .unwrap();
        app.handle_action(Action::MessageAppend {
            agent_id: session_id.clone(),
            text: "Hello! How can I help?".to_string(),
        })
        .await
        .unwrap();

        // 4. sync_components 将消息同步到主视图
        app.sync_components();

        // 验证：主视图包含用户消息和 AI 回复
        assert_eq!(
            app.tui.main_view.messages.len(),
            2,
            "主视图应该包含用户消息和 AI 回复"
        );
        assert!(matches!(
            app.tui.main_view.messages[0].role,
            crate::message::ChatRole::User
        ));
        assert!(matches!(
            app.tui.main_view.messages[1].role,
            crate::message::ChatRole::Assistant
        ));
        assert!(!app.tui.main_view.messages[1].text.is_empty());
        assert_eq!(app.tui.main_view.messages[1].content.len(), 1);
    }

    #[tokio::test]
    async fn thinking_delta_should_build_content_blocks_for_streaming_render() {
        let mut app = App::new_rpc(tokio::sync::mpsc::channel::<Action>(1).0);
        let agent_id = app.active_agent.clone().unwrap();

        app.handle_action(Action::ThinkingAppend {
            agent_id: agent_id.clone(),
            text: "先分析问题。\n".to_string(),
        })
        .await
        .unwrap();
        app.handle_action(Action::ThinkingAppend {
            agent_id: agent_id.clone(),
            text: "再给出修复。".to_string(),
        })
        .await
        .unwrap();
        app.handle_action(Action::MessageAppend {
            agent_id: agent_id.clone(),
            text: "最终回复".to_string(),
        })
        .await
        .unwrap();

        let msg = app.messages.get(&agent_id).unwrap().last().unwrap();
        assert_eq!(msg.content.len(), 2);
        assert!(matches!(
            &msg.content[0],
            crate::message::ContentBlock::Thinking { thinking }
                if thinking == "先分析问题。\n再给出修复。"
        ));
        assert!(matches!(
            &msg.content[1],
            crate::message::ContentBlock::Text { text } if text == "最终回复"
        ));
        assert_eq!(app.tui.main_view.messages[0].content.len(), 2);
    }

    #[tokio::test]
    async fn content_update_should_sync_main_view_immediately() {
        let mut app = App::new_rpc(tokio::sync::mpsc::channel::<Action>(1).0);
        let agent_id = app.active_agent.clone().unwrap();

        app.handle_action(Action::ContentUpdate {
            agent_id: agent_id.clone(),
            content: vec![crate::message::ContentBlock::Thinking {
                thinking: "完整思考快照".to_string(),
            }],
        })
        .await
        .unwrap();

        assert_eq!(app.tui.main_view.messages.len(), 1);
        assert!(matches!(
            &app.tui.main_view.messages[0].content[0],
            crate::message::ContentBlock::Thinking { thinking }
                if thinking == "完整思考快照"
        ));
    }

    #[tokio::test]
    async fn thinking_finalize_should_keep_streamed_text_when_delta_is_empty() {
        let mut app = App::new_rpc(tokio::sync::mpsc::channel::<Action>(1).0);
        let agent_id = app.active_agent.clone().unwrap();

        app.handle_action(Action::ThinkingAppend {
            agent_id: agent_id.clone(),
            text: "已经流式收到的思考".to_string(),
        })
        .await
        .unwrap();
        app.handle_action(Action::ThinkingFinalize {
            agent_id: agent_id.clone(),
            text: String::new(),
        })
        .await
        .unwrap();

        let msg = app.messages.get(&agent_id).unwrap().last().unwrap();
        assert_eq!(msg.thinking.as_deref(), Some("已经流式收到的思考"));
    }

    #[tokio::test]
    async fn thinking_append_after_tool_should_create_tail_assistant_message() {
        let mut app = App::new_rpc(tokio::sync::mpsc::channel::<Action>(1).0);
        let agent_id = app.active_agent.clone().unwrap();

        app.handle_action(Action::ThinkingAppend {
            agent_id: agent_id.clone(),
            text: "工具前思考".to_string(),
        })
        .await
        .unwrap();
        app.handle_action(Action::ToolEvent {
            agent_id: agent_id.clone(),
            tool_name: "bash".to_string(),
            tool_call_id: "tc-tail".to_string(),
            status: crate::message::ToolStatus::Running,
            args: Some(serde_json::json!({"command": "echo ok"})),
            result: None,
            is_error: false,
        })
        .await
        .unwrap();
        app.handle_action(Action::ThinkingAppend {
            agent_id: agent_id.clone(),
            text: "工具后思考".to_string(),
        })
        .await
        .unwrap();

        let msgs = app.messages.get(&agent_id).unwrap();
        assert_eq!(msgs.len(), 3);
        assert!(matches!(msgs[0].role, crate::message::ChatRole::Assistant));
        assert!(matches!(msgs[1].role, crate::message::ChatRole::Tool));
        assert!(matches!(msgs[2].role, crate::message::ChatRole::Assistant));
        assert_eq!(msgs[0].thinking.as_deref(), Some("工具前思考"));
        assert_eq!(msgs[2].thinking.as_deref(), Some("工具后思考"));
        assert_eq!(app.tui.main_view.messages.len(), 3);
    }

    #[tokio::test]
    async fn message_append_after_tool_should_create_tail_assistant_message() {
        let mut app = App::new_rpc(tokio::sync::mpsc::channel::<Action>(1).0);
        let agent_id = app.active_agent.clone().unwrap();

        app.handle_action(Action::MessageAppend {
            agent_id: agent_id.clone(),
            text: "工具前回复".to_string(),
        })
        .await
        .unwrap();
        app.handle_action(Action::ToolEvent {
            agent_id: agent_id.clone(),
            tool_name: "bash".to_string(),
            tool_call_id: "tc-text-tail".to_string(),
            status: crate::message::ToolStatus::Running,
            args: Some(serde_json::json!({"command": "echo ok"})),
            result: None,
            is_error: false,
        })
        .await
        .unwrap();
        app.handle_action(Action::MessageAppend {
            agent_id: agent_id.clone(),
            text: "工具后回复".to_string(),
        })
        .await
        .unwrap();

        let msgs = app.messages.get(&agent_id).unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].text, "工具前回复");
        assert!(matches!(msgs[1].role, crate::message::ChatRole::Tool));
        assert_eq!(msgs[2].text, "工具后回复");
    }

    #[tokio::test]
    async fn content_update_after_tool_should_create_tail_assistant_message() {
        let mut app = App::new_rpc(tokio::sync::mpsc::channel::<Action>(1).0);
        let agent_id = app.active_agent.clone().unwrap();

        app.handle_action(Action::ContentUpdate {
            agent_id: agent_id.clone(),
            content: vec![crate::message::ContentBlock::Thinking {
                thinking: "工具前快照".to_string(),
            }],
        })
        .await
        .unwrap();
        app.handle_action(Action::ToolEvent {
            agent_id: agent_id.clone(),
            tool_name: "bash".to_string(),
            tool_call_id: "tc-content-tail".to_string(),
            status: crate::message::ToolStatus::Running,
            args: Some(serde_json::json!({"command": "echo ok"})),
            result: None,
            is_error: false,
        })
        .await
        .unwrap();
        app.handle_action(Action::ContentUpdate {
            agent_id: agent_id.clone(),
            content: vec![crate::message::ContentBlock::Thinking {
                thinking: "工具后快照".to_string(),
            }],
        })
        .await
        .unwrap();

        let msgs = app.messages.get(&agent_id).unwrap();
        assert_eq!(msgs.len(), 3);
        assert!(matches!(
            &msgs[0].content[0],
            crate::message::ContentBlock::Thinking { thinking } if thinking == "工具前快照"
        ));
        assert!(matches!(msgs[1].role, crate::message::ChatRole::Tool));
        assert!(matches!(
            &msgs[2].content[0],
            crate::message::ContentBlock::Thinking { thinking } if thinking == "工具后快照"
        ));
    }

    #[tokio::test]
    async fn thinking_start_should_begin_new_content_block() {
        let mut app = App::new_rpc(tokio::sync::mpsc::channel::<Action>(1).0);
        let agent_id = app.active_agent.clone().unwrap();

        app.handle_action(Action::ThinkingAppend {
            agent_id: agent_id.clone(),
            text: "第一段思考".to_string(),
        })
        .await
        .unwrap();
        app.handle_action(Action::ThinkingStart {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();
        app.handle_action(Action::ThinkingAppend {
            agent_id: agent_id.clone(),
            text: "第二段思考".to_string(),
        })
        .await
        .unwrap();

        let msg = app.messages.get(&agent_id).unwrap().last().unwrap();
        assert_eq!(msg.content.len(), 2);
        assert!(matches!(
            &msg.content[0],
            crate::message::ContentBlock::Thinking { thinking } if thinking == "第一段思考"
        ));
        assert!(matches!(
            &msg.content[1],
            crate::message::ContentBlock::Thinking { thinking } if thinking == "第二段思考"
        ));
    }

    #[tokio::test]
    async fn thinking_start_after_tool_should_create_tail_assistant_message() {
        let mut app = App::new_rpc(tokio::sync::mpsc::channel::<Action>(1).0);
        let agent_id = app.active_agent.clone().unwrap();

        app.handle_action(Action::ThinkingAppend {
            agent_id: agent_id.clone(),
            text: "工具前思考".to_string(),
        })
        .await
        .unwrap();
        app.handle_action(Action::ToolEvent {
            agent_id: agent_id.clone(),
            tool_name: "bash".to_string(),
            tool_call_id: "tc-thinking-start-tail".to_string(),
            status: crate::message::ToolStatus::Running,
            args: Some(serde_json::json!({"command": "echo ok"})),
            result: None,
            is_error: false,
        })
        .await
        .unwrap();
        app.handle_action(Action::ThinkingStart {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();
        app.handle_action(Action::ThinkingAppend {
            agent_id: agent_id.clone(),
            text: "工具后思考".to_string(),
        })
        .await
        .unwrap();

        let msgs = app.messages.get(&agent_id).unwrap();
        assert_eq!(msgs.len(), 3);
        assert!(matches!(msgs[0].role, crate::message::ChatRole::Assistant));
        assert!(matches!(msgs[1].role, crate::message::ChatRole::Tool));
        assert!(matches!(msgs[2].role, crate::message::ChatRole::Assistant));
        assert_eq!(msgs[2].thinking.as_deref(), Some("工具后思考"));
    }

    /// 测试：CreateSession 的前置逻辑 —— workspace 查找、session 路径生成
    ///
    /// 不依赖实际 pi 进程，验证在 workspace 存在时能正确生成 session 路径和 ID。
    #[tokio::test]
    async fn test_create_session_preamble() {
        let mut app = App::new_rpc(tokio::sync::mpsc::channel::<Action>(1).0);
        // 手动构造一个工作区（模拟 populate_workspaces 的输出）
        let ws = WorkspaceNode {
            cwd: "/home/hr/Projects/agent-tui".to_string(),
            display_name: "agent-tui".to_string(),
            sessions: vec![],
            expanded: true,
        };
        app.tui.workspaces = vec![ws];
        app.tui.persistence_disabled = true;

        // 设置 event_tx（模拟 run_tui 初始化）
        let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();
        app.event_tx = Some(event_tx);

        // 执行 CreateSession（会尝试 spawn pi，在测试环境中会失败）
        // 但我们可以通过检查 spawn 失败后的状态来验证前置逻辑
        let result = app
            .handle_action(Action::CreateSession {
                workspace_index: 0,
                name: "测试会话".to_string(),
            })
            .await;

        // spawn 可能成功也可能失败，取决于是否有 pi CLI
        // 至少验证：名称被存入了 session_names
        if let Some(sid) = &app.active_agent {
            assert!(
                app.tui.session_names.contains_key(sid),
                "session_names 应包含新建 session 的名称"
            );
            assert_eq!(app.tui.session_names.get(sid).unwrap(), "测试会话");
            // 验证 session 被加入 active_sessions
            assert!(app.active_sessions.contains(sid));
        }
        // 无论如何不应该 panic
        let _ = result;
    }
}

#[test]
fn test_extract_session_name_from_session_name_line() {
    // pi 自动创建的 session 第一行有 type=session + id + cwd，无 session_meta
    // 第二位若有 type=session_name 行则优先使用其 name
    let dir = std::env::temp_dir().join(format!("tui_test_session_name_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("test.jsonl");
    std::fs::write(
            &path,
            r#"{"type":"session","version":3,"id":"019eea06-e08e-7057-8793-8bedab78eaf4","timestamp":"2026-06-21T11:52:59.790Z","cwd":"/home/hr/.config/pi-desktop/chat-workspace"}
{"type":"session_name","name":"chat-workspace-调试","version":3}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"你好"}]}}
"#,
        )
        .unwrap();

    let name = crate::utils::extract_session_name(&path);
    assert_eq!(name.as_deref(), Some("chat-workspace-调试"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_extract_session_name_from_user_message() {
    // 无 session_name 行时，取第一条 user 消息的前 35 字
    let dir = std::env::temp_dir().join(format!("tui_test_session_msg_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("test.jsonl");
    std::fs::write(
            &path,
            r#"{"type":"session","version":3,"id":"019e3b77-9ea9-7656-b707-45b1f2363e7f","timestamp":"2026-05-18T14:22:35.689Z","cwd":"/home/hr"}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"帮我分析一下这个项目的架构"}]}}
{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"好的，让我看看"}]}}
"#,
        )
        .unwrap();

    let name = crate::utils::extract_session_name(&path);
    assert_eq!(name.as_deref(), Some("帮我分析一下这个项目的架构"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_extract_session_name_no_names_returns_none() {
    // 只有 type=session 无 session_name 也无 user 消息 → None
    let dir = std::env::temp_dir().join(format!("tui_test_session_none_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("test.jsonl");
    std::fs::write(
        &path,
        r#"{"type":"session","version":3,"id":"abc123","cwd":"/tmp"}
{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"你好"}]}}
"#,
    )
    .unwrap();

    let name = crate::utils::extract_session_name(&path);
    assert!(name.is_none());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_extract_session_name_long_user_message_truncated() {
    // 超过 35 字符的 user 消息被截断
    let dir = std::env::temp_dir().join(format!("tui_test_session_long_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("test.jsonl");
    std::fs::write(
            &path,
            r#"{"type":"session","version":3,"id":"abc123","cwd":"/tmp"}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"这是一个非常非常非常长的用户问题用来测试会话名称截断逻辑是否正常工作666666"}]}}
"#,
        )
        .unwrap();

    let name = crate::utils::extract_session_name(&path);
    assert!(name.is_some());
    let n = name.unwrap();
    // 截断后应有 … 后缀
    assert!(
        n.ends_with('…'),
        "long name should be truncated with …: {}",
        n
    );
    // 截断字符长度 ≤ 35 + 1（…）
    assert!(
        n.chars().count() <= 36,
        "truncated name too long: {} chars",
        n.chars().count()
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_extract_session_name_skip_non_user_message() {
    // 跳过 assistant/tool 消息，直到找到 user 消息
    let dir = std::env::temp_dir().join(format!("tui_test_session_skip_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("test.jsonl");
    std::fs::write(
        &path,
        r#"{"type":"session","version":3,"id":"abc123","cwd":"/tmp"}
{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"跳过这条"}]}}
{"type":"message","message":{"role":"tool","content":[]}}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"这才是用户消息"}]}}
"#,
    )
    .unwrap();

    let name = crate::utils::extract_session_name(&path);
    assert_eq!(name.as_deref(), Some("这才是用户消息"));
    let _ = std::fs::remove_file(&path);
}
