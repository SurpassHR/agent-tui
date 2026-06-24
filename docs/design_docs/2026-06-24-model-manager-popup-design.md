# 模型管理弹窗设计

## 问题

编辑 Provider 弹窗中，模型字段存在两个问题：

1. **模型显示不全** — 渲染代码使用 `.take(3)` 只显示前 3 行模型
2. **无法编辑模型** — 焦点在模型字段时，Up/Down 切换的是表单字段焦点而非模型列表导航；`models_text` 仅是 `String`，无行内光标概念

## 解决方案

在 Provider 编辑弹窗中，模型字段改为**摘要行**显示，通过 Enter 键弹出**独立的模型管理子弹窗**进行浏览/编辑/删除/添加操作。

### 交互流程

```
Provider 编辑表单
    │
    │  模型字段显示摘要行：
    │  「gpt-4o:T3:128000, gpt-4o-mini:T1:128000 … (+N 个模型)」
    │
    ├── Enter (当 field_focus == 5) → 模型管理弹窗（子弹窗 1）
    │            │
    │            │  进入前：将 models_text 解析合并到 draft.models
    │            │  List widget 显示所有模型（每行 id:tier:ctx）
    │            │  最后一行：「+ 添加模型」
    │            │
    │            ├── Enter (选中某行) → 模型编辑弹窗（子弹窗 2）
    │            │                       三个字段：ID / Tier / Context Window
    │            │                       Char/Backspace 编辑字段内容
    │            │                       Enter 确认 / Esc 取消
    │            │
    │            ├── Delete/Backspace → 删除选中模型（无二次确认）
    │            └── Esc → 关闭管理弹窗，同步 models_text ← rebuild(draft.models)
    │                      返回 Provider 编辑表单
    │
    └── 其他字段照常编辑（ID / Name / Base URL / API Key / Mode）
```

### 数据流变化

```
Provider 编辑表单（已有）
    │
    ├── field_focus != 5（非模型字段）
    │       → 现有表单编辑逻辑，不变
    │
    └── field_focus == 5（模型字段）
            → 显示摘要行（不可编辑文本）
            → Enter 进入 model_manager 子状态（在保存分支之前拦截）
```

```
模型管理弹窗（新增）
    │
    ├── 浏览模式（manager.editor == None）
    │       → List widget + ListState，Up/Down 导航，选定项始终可见
    │       → Enter: 选中模型行 → 打开三级编辑；选中添加行 → 新建空模型并打开编辑
    │       → Delete/Backspace: 删除选中模型行
    │       → Esc: 退出，rebuild_models_text() 同步回 models_text
    │       → Tab: 无操作（保持焦点在列表内）
    │
    └── 编辑模式（manager.editor == Some）
            → 三级弹窗，Tab/Up/Down 切换 id/tier/ctx 字段焦点
            → Char(c): 追加到当前聚焦字段的 draft 字符串
            → Backspace: 从当前聚焦字段的 draft 字符串末尾删除
            → Enter: 验证并写回 draft.models[idx]，editor = None（返浏览模式）
            → Esc: 取消修改，editor = None（不修改 draft.models）
```

### 状态结构变更

```rust
/// Provider 编辑状态
pub struct ProviderEditor {
    // 现有字段不变
    pub is_new: bool,
    pub index: usize,
    pub draft: crate::provider::ProviderInfo,
    pub field_focus: usize,
    pub models_text: String,
    pub models_fetching: bool,

    // 新增
    pub model_mgr: Option<ModelManager>,
}

/// 模型管理弹窗状态
pub struct ModelManager {
    /// 列表中光标位置（0..N-1 为模型行，N = "添加模型"行）
    pub cursor: usize,
    /// 编辑单个模型的状态（None=浏览列表，Some=编辑字段）
    pub editor: Option<ModelFieldEditor>,
}

/// 三级模型字段编辑弹窗
pub struct ModelFieldEditor {
    /// 当前聚焦字段 (0=ID, 1=Tier, 2=Context)
    pub field_focus: usize,
    /// 编辑草稿
    pub draft_id: String,
    pub draft_tier: String,
    pub draft_ctx: String,
    /// 被编辑的模型在 Vec 中的索引
    pub model_index: usize,
}

/// 辅助函数：从 draft.models 重建 models_text
fn rebuild_models_text(models: &[crate::provider::ModelInfo]) -> String {
    models.iter()
        .map(|m| format!("{}:{}:{}", m.id, m.tier, m.context_window))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 辅助函数：将 models_text 解析合并到 draft.models
/// 覆盖：models_text 行完全替换 draft.models
/// 保留：models_text 为空时不作修改
fn merge_models_text(text: &str, existing: &[crate::provider::ModelInfo]) -> Vec<crate::provider::ModelInfo> {
    let parsed: Vec<crate::provider::ModelInfo> = text
        .split('\n')
        .filter(|s| !s.trim().is_empty())
        .filter_map(|entry| {
            let parts: Vec<&str> = entry.trim().split(':').collect();
            let id = parts.first()?.trim().to_string();
            if id.is_empty() { return None; }
            let tier = parts.get(1).map(|s| s.trim()).unwrap_or("T2").to_string();
            let ctx = parts.get(2).and_then(|s| s.trim().parse().ok()).unwrap_or(128000);
            Some(crate::provider::ModelInfo {
                id,
                name: String::new(),
                context_window: ctx,
                reasoning: tier != "T1",
                tier,
            })
        })
        .collect();
    if parsed.is_empty() { existing.to_vec() } else { parsed }
}
```

### 键盘处理分层

```
handle_provider_editor_key(key)
    │
    ├── editor.model_mgr.is_some()
    │       → handle_model_manager_key(key)
    │           │
    │           ├── manager.editor.is_some()
    │           │       → handle_model_field_editor_key(key)
    │           │           Up/Down/Tab → 切换 field_focus (0/1/2)
    │           │           Char(c) → 追加到当前字段的 draft 字符串
    │           │           Backspace → 从当前字段的 draft 字符串末尾 pop
    │           │           Enter → 验证（tier∈{T1,T2,T3}, ctx∈u32>0），
    │           │                   写回 draft.models[model_index]，editor = None
    │           │           Esc → 取消修改，editor = None（不修改数据）
    │           │
    │           └── manager.editor.is_none()
    │                   Up/Down → cursor ± 1，限制 [0, draft.models.len()]
    │                   Enter → cursor 在模型行时 editor = Some(...)，
    │                           在添加行时 push 默认模型并 editor = Some(...)
    │                   Delete/Backspace → 删除 draft.models[cursor]，
    │                     cursor = min(cursor, models.len())，若空则 cursor = 0
    │                   Tab → 无操作（已消费，不穿透）
    │                   Esc → model_mgr = None，同步 models_text = rebuild(...)
    │
    └── editor.model_mgr.is_none()
            → 现有表单键处理，但在保存（Enter）之前加拦截：
              若 field_focus == 5 → 打开模型管理器：
                model_mgr = Some(ModelManager { cursor: 0, editor: None })
                同时将 models_text 解析合并到 draft.models
                不执行保存
            → 其他字段 Enter 照常保存
```

### models_text 同步规则

| 事件 | 操作 |
|------|------|
| 打开模型管理器 | `merge_models_text(editor.models_text, &editor.draft.models)` → 写入 `editor.draft.models` |
| 编辑/删除/添加模型 | 直接修改 `draft.models` |
| 关闭模型管理器 (Esc) | `rebuild_models_text(&editor.draft.models)` → 写入 `editor.models_text` |
| API 拉取完成 | `models_text = 拉取文本`（保留现有逻辑） |
| 表单保存 (Enter) | `models_text` 解析写入 `draft.models` 的现有逻辑保留不变 |

### 模型编辑弹窗验证规则

| 字段 | 验证 | 失败处理 |
|------|------|----------|
| ID | 非空 | 不允许确认，焦点留在 ID 字段 |
| Tier | `T1` / `T2` / `T3`（大小写不敏感） | 不允许确认，焦点留在 Tier 字段 |
| Context | 可解析为 u32 且 > 0 | 不允许确认，焦点留在 Context 字段 |

- 验证失败时不写入 `draft.models`
- 三级弹窗保持打开状态，用户修正后重新 Enter

### 渲染结构

#### 模型字段摘要行（表单中，field_focus == 5 时高亮）

```
  ▎模型（每行一个，格式：id:tier:contextWindow）
  ┌──────────────────────────────────────────────────┐
  │ gpt-4o:T3:128000, gpt-4o-mini:T1… (+6 个模型)   │
  └──────────────────────────────────────────────────┘
```

- `models_text.lines().count()` > 1 时显示 `... (+N个模型)`
- 默认显示首行摘要，首行太长时截断

#### 模型管理弹窗

```
┌─ 管理模型 ──────────────────────────────┐
│  ▶ gpt-4o          T3 · 128,000        │  ← ListState selected 高亮
│    gpt-4o-mini      T1 · 128,000        │
│    o1               T3 · 200,000        │
│    o3               T3 · 100,000        │
│    claude-sonnet-4  T3 · 200,000        │
│    claude-haiku-3   T1 · 200,000        │
│    ─────────────────────────────────── │
│    + 添加模型                          │
├────────────────────────────────────────┤
│  ↑↓ 移动  Enter 编辑  Del 删除  Esc 返回 │
└────────────────────────────────────────┘
```

- 使用 Ratatui `List` + `ListState` widget
- `ModelManager` 中持久化 `ListState`（`selected` + `offset`），每帧渲染后自动维护滚动
- 弹窗宽 55%，高度按模型数量动态（最多 70% 视口）
- 列表项格式：左侧 `id`（含 CJK 宽度处理），右侧 `tier · ctx`

#### 三级模型编辑弹窗

```
┌─ 编辑模型 ───────────────┐
│  ▎ID                     │
│  ┌──────────────────────┐│
│  │ gpt-4o              ││
│  └──────────────────────┘│
│  ▎Tier                   │
│  ┌──────────────────────┐│
│  │ T3                  ││
│  └──────────────────────┘│
│  ▎Context Window         │
│  ┌──────────────────────┐│
│  │ 128000              ││
│  └──────────────────────┘│
│  [✓ 确认] [取消] Tab Enter Esc │
└──────────────────────────┘
```

- 弹窗宽 40%，高固定（约 14 行）
- 居中于模型管理弹窗之上

### 需要修改的文件

| 文件 | 改动 |
|------|------|
| `src/app.rs` | 添加 `ModelManager`、`ModelFieldEditor` 结构体 + 辅助函数；修改 `ProviderEditor` 添加 `model_mgr` 字段；修改 `handle_provider_editor_key`：Enter 时拦截 field_focus==5、Up/Down/Tab 在 field==5 时跳过、添加模型管理器/编辑器的键盘处理分支；修改模型字段渲染为摘要行；添加模型管理弹窗和编辑弹窗的渲染代码 |
| `src/tui.rs` | 无需修改（事件分派已通过 `handle_provider_key` → `handle_provider_editor_key` 路由） |
| `src/provider/mod.rs` | 无需修改 |

### 边界情况

- **模型列表为空**：摘要行显示 `(空)`，管理弹窗只显示 `+ 添加模型`
- **模型数量很多**（>20）：`ListState` 自带滚动，弹窗高度封顶
- **删除最后一个模型**：cursor 回退到 0（添加行）；`draft.models` 为空列表
- **编辑时 Esc 取消**：不修改 `draft.models`，`editor = None`
- **Enter 确认编辑**：验证通过后写回 `draft.models[idx]`，`editor = None`
- **Tab 键穿透防护**：模型管理弹窗浏览模式下 Tab 被消费但不做操作，不穿透到表单层
- **Esc 事件消费**：模型管理弹窗和编辑弹窗的 Esc 均返回 `true`，防止穿透关闭整个编辑器
- **自动拉取后的状态**：拉取结果写入 `models_text`；进入管理弹窗时 `merge_models_text` 将其解析到 `draft.models`
- **API 拉取 + 已有编辑数据冲突**：`merge_models_text` 在 text 非空时完全覆盖 `draft.models`（即拉取结果优先）。用户如果已在表单中手动编辑过模型，需注意这一行为
- **ModelInfo.name/reasoning 字段**：三级编辑弹窗不展示这两个字段。编辑后 `name` 置为空字符串，`reasoning` 从 tier 推导（tier == "T1" → false，其他 → true）
- **Unicode 宽度**：模型 ID 可能包含 CJK 字符，`List` item 渲染需使用 `unicode-width` crate 计算列对齐
