//! Section paradigm (`SectionContext`, spec §7.1).
//!
//! Three-level model (Message ⊂ turn ⊂ section), three-zone window partition,
//! standard label set, the `record_section` boundary tool, automatic
//! compression modes, and the paradigm-owned persistence orchestration.
//!
//! Implementation notes on spec interpretation:
//! - Cache-zone membership is decided at creation: the first
//!   `cache_prefix_sections` sections ever created are cache-zone forever
//!   (frozen Raw, never compressed or reordered).
//! - The newest section is always the active write target; additional active
//!   sections follow the `ActiveWindow`. Sealed/cache sections never
//!   contribute turns to a `record_section` carve ("只合并 Open 部分").
//! - `record_section` carving: a boundary range that is exactly the newest
//!   section is idempotent (label-only); a proper subset is carved out as a
//!   new section; a range spanning open sections merges them into one new
//!   section.
//! - `SectionSwitch` triggers evaluation after a carve; the target is the best
//!   sealed candidate (priority desc, oldest first). `Hyper` targets the
//!   just-carved newest section unconditionally (active window 0), still
//!   honoring the Raw + compressible-label hard constraints.
//! - Persistence (state / original / compressed) only happens after
//!   `restore_session` provides a session id; without one the instance stays
//!   in-memory only.
//! - `decompress` restores the original as a single turn (the stored original
//!   is flattened per spec §7.1.7).

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::SystemTime;

use armillae_core::{ContentPart, Message, Role, TokenUsage, ToolDefinition};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::context::Context;
use crate::error::ContextError;
use crate::machine::CompressionMachine;
use crate::protocol::{CompressionState, CompressionTarget};
use crate::store::{
    CompressedRef, OriginalRef, OriginalSnapshot, SECTION_STATE_SCHEMA_VERSION,
    SectionCompressedEntry, SectionOriginalEntry, SectionState, SectionStore, TokenFacts,
    WindowState,
};

/// Namespaced paradigm identifier of the section paradigm (spec §4.2).
pub const PARADIGM_ID: &str = "armillae-context/section";

/// Name of the `record_section` boundary tool (spec §7.1.3).
const RECORD_SECTION_TOOL_NAME: &str = "context.record_section";

/// Standard labels (spec §7.1.5).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum StandardLabel {
    Plan,
    Constraint,
    Preference,
    Decision,
    Fact,
    Task,
    ToolExecution,
    Dialog,
    Uncategorized,
}

impl StandardLabel {
    /// Wire name used by the `record_section` tool schema.
    pub fn wire_name(self) -> &'static str {
        match self {
            StandardLabel::Plan => "plan",
            StandardLabel::Constraint => "constraint",
            StandardLabel::Preference => "preference",
            StandardLabel::Decision => "decision",
            StandardLabel::Fact => "fact",
            StandardLabel::Task => "task",
            StandardLabel::ToolExecution => "tool_execution",
            StandardLabel::Dialog => "dialog",
            StandardLabel::Uncategorized => "uncategorized",
        }
    }

    fn from_wire_name(name: &str) -> Option<Self> {
        Some(match name {
            "plan" => StandardLabel::Plan,
            "constraint" => StandardLabel::Constraint,
            "preference" => StandardLabel::Preference,
            "decision" => StandardLabel::Decision,
            "fact" => StandardLabel::Fact,
            "task" => StandardLabel::Task,
            "tool_execution" => StandardLabel::ToolExecution,
            "dialog" => StandardLabel::Dialog,
            "uncategorized" => StandardLabel::Uncategorized,
            _ => return None,
        })
    }

    fn iter_wire_names() -> impl Iterator<Item = &'static str> {
        [
            StandardLabel::Plan,
            StandardLabel::Constraint,
            StandardLabel::Preference,
            StandardLabel::Decision,
            StandardLabel::Fact,
            StandardLabel::Task,
            StandardLabel::ToolExecution,
            StandardLabel::Dialog,
            StandardLabel::Uncategorized,
        ]
        .into_iter()
        .map(StandardLabel::wire_name)
    }
}

/// Section label: a standard label or a namespaced custom label (spec §4.2).
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum SectionLabel {
    Standard(StandardLabel),
    Custom(String),
}

/// Compression depth used by the compression instruction (spec §7.1.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompressionMethod {
    Shallow,
    Deep,
}

/// Label policy: compressibility, priority, and depth (spec §7.1.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LabelPolicy {
    /// Whether sections with this label may be compressed (hard constraint).
    pub compressible: bool,
    /// Compression priority; higher values are compressed first.
    pub priority: u8,
    pub method: CompressionMethod,
}

/// Policy for tool turns inside a compression candidate (spec §7.1.0).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToolTurnPolicy {
    /// Allow compression; tool turns degrade to natural-language summaries.
    Downgrade,
    /// Exclude sections containing tool turns from compression candidates.
    Reject,
}

/// Active window mode (spec §7.1.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActiveWindow {
    /// Keep the newest `count` sections active (1 minimum).
    Sections { count: usize },
    /// The whole dynamic zone is active; automatic compression is disabled.
    All,
    /// Active window 0; every just-carved section is compressed immediately.
    Hyper,
}

/// Automatic compression mode (spec §7.1.5; `None` = manual only).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AutoCompression {
    TokenThreshold { threshold: u64 },
    SectionSwitch,
}

/// Section-paradigm configuration; immutable after build (spec §7.1.5).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SectionConfig {
    /// Cache-zone section count (determined at creation, never changes).
    pub cache_prefix_sections: usize,
    pub active_window: ActiveWindow,
    /// `None` = manual compression only.
    pub auto_compression: Option<AutoCompression>,
    pub tool_turn_policy: ToolTurnPolicy,
    /// Role of exported summary messages (spec §8.1; default User).
    pub compressed_message_role: Role,
    /// Target output tokens for the compression instruction (`None` = default).
    pub compression_token_target: Option<u64>,
    /// Label policies; the standard set is the default (spec §7.1.5).
    pub label_policies: BTreeMap<StandardLabel, LabelPolicy>,
}

impl Default for SectionConfig {
    fn default() -> Self {
        Self {
            cache_prefix_sections: 0,
            active_window: ActiveWindow::Sections { count: 4 },
            auto_compression: None,
            tool_turn_policy: ToolTurnPolicy::Downgrade,
            compressed_message_role: Role::User,
            compression_token_target: None,
            label_policies: standard_label_policies(),
        }
    }
}

fn standard_label_policies() -> BTreeMap<StandardLabel, LabelPolicy> {
    use CompressionMethod::{Deep, Shallow};
    use StandardLabel::*;
    let never = LabelPolicy {
        compressible: false,
        priority: 0,
        method: Shallow,
    };
    BTreeMap::from([
        (Plan, never),
        (Constraint, never),
        (Preference, never),
        (
            Decision,
            LabelPolicy {
                compressible: true,
                priority: 1,
                method: Shallow,
            },
        ),
        (
            Fact,
            LabelPolicy {
                compressible: true,
                priority: 0,
                method: Deep,
            },
        ),
        (
            Task,
            LabelPolicy {
                compressible: true,
                priority: 0,
                method: Deep,
            },
        ),
        (
            ToolExecution,
            LabelPolicy {
                compressible: true,
                priority: 0,
                method: Deep,
            },
        ),
        (
            Dialog,
            LabelPolicy {
                compressible: true,
                priority: 0,
                method: Deep,
            },
        ),
        (
            Uncategorized,
            LabelPolicy {
                compressible: true,
                priority: 0,
                method: Deep,
            },
        ),
    ])
}

/// Section view: original or compressed summary (spec §7.1.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum View {
    Raw,
    Compressed,
}

/// One complete dialogue turn (user input → final output, including
/// intermediate tool-call rounds); the write/accumulation atomic unit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Turn {
    pub messages: Vec<Message>,
}

/// A section: a run of consecutive same-boundary turns with a label, view,
/// and compression state (spec §7.1.1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Section {
    pub id: u64,
    pub label: SectionLabel,
    pub view: View,
    /// Original turns; empty while the view is `Compressed`.
    pub turns: Vec<Turn>,
    pub version: u64,
    pub original_ref: Option<OriginalRef>,
    pub compressed_ref: Option<CompressedRef>,
    /// In-memory compressed view; not persisted (loaded from the store).
    #[serde(skip)]
    pub summary: Option<Vec<Message>>,
}

impl Section {
    fn new(id: u64, label: SectionLabel) -> Self {
        Self {
            id,
            label,
            view: View::Raw,
            turns: Vec::new(),
            version: 0,
            original_ref: None,
            compressed_ref: None,
            summary: None,
        }
    }
}

/// Observable mapping record of a section (spec §7.1.6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MappingRecord {
    pub id: u64,
    pub label: SectionLabel,
    pub view: View,
    pub version: u64,
    pub turn_count: usize,
}

fn mapping_record(section: &Section) -> MappingRecord {
    MappingRecord {
        id: section.id,
        label: section.label.clone(),
        view: section.view,
        version: section.version,
        turn_count: section.turns.len(),
    }
}

/// Builder for `SectionContext`; label overrides and custom labels must be
/// injected before `build()` freezes the mapping and the tool schema.
pub struct SectionContextBuilder {
    config: SectionConfig,
    store: Arc<dyn SectionStore>,
    custom_labels: BTreeMap<String, LabelPolicy>,
}

impl SectionContextBuilder {
    /// Override the policy of a standard label (spec §7.1.5).
    pub fn with_policy(mut self, label: StandardLabel, policy: LabelPolicy) -> Self {
        self.config.label_policies.insert(label, policy);
        self
    }

    /// Register a namespaced custom label with its policy (spec §4.2).
    pub fn with_custom_label(mut self, name: impl Into<String>, policy: LabelPolicy) -> Self {
        self.custom_labels.insert(name.into(), policy);
        self
    }

    /// Build the paradigm; validates labels and freezes the mapping and the
    /// `record_section` tool schema.
    pub fn build(self) -> Result<SectionContext, ContextError> {
        for name in self.custom_labels.keys() {
            if name.is_empty() {
                return Err(ContextError::InvalidConfiguration {
                    message: "custom label must not be empty".to_owned(),
                });
            }
            if !is_namespaced(name) {
                return Err(ContextError::InvalidConfiguration {
                    message: format!(
                        "custom label '{name}' must be a namespaced string (e.g. custom.xxx)"
                    ),
                });
            }
        }
        let tool_definition =
            record_section_definition(&self.custom_labels.keys().cloned().collect::<Vec<_>>());
        let context = SectionContext {
            config: self.config,
            custom_label_policies: self.custom_labels,
            store: self.store,
            machine: CompressionMachine::new(),
            session_id: None,
            sections: Vec::new(),
            next_section_id: 0,
            token_facts: TokenFacts::default(),
            tool_definition,
            last_event_carved_section: false,
            pending_original: None,
        };
        Ok(context)
    }
}

fn is_namespaced(name: &str) -> bool {
    name.contains('.') && !name.chars().any(char::is_whitespace)
}

fn record_section_definition(custom_labels: &[String]) -> ToolDefinition {
    let mut labels: Vec<&str> = StandardLabel::iter_wire_names().collect();
    for custom in custom_labels {
        labels.push(custom);
    }
    ToolDefinition {
        name: RECORD_SECTION_TOOL_NAME.to_owned(),
        description: "每次回答结束后调用：划定最新一小节的起始边界，并可选标注其标签".to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "section_start_rounds": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "最新小节从最近第几轮完整对话开始（1=仅最新一轮自成小节）"
                },
                "label": {
                    "type": "string",
                    "enum": labels,
                    "description": "小节标签（可选）"
                }
            },
            "required": ["section_start_rounds"]
        }),
    }
}

/// Section paradigm: session-level state container driven serially by its
/// downstream (spec §7.1, §10).
pub struct SectionContext {
    config: SectionConfig,
    custom_label_policies: BTreeMap<String, LabelPolicy>,
    store: Arc<dyn SectionStore>,
    machine: CompressionMachine,
    session_id: Option<String>,
    sections: Vec<Section>,
    next_section_id: u64,
    token_facts: TokenFacts,
    tool_definition: ToolDefinition,
    last_event_carved_section: bool,
    pending_original: Option<(u64, OriginalRef)>,
}

impl SectionContext {
    pub fn builder(config: SectionConfig, store: Arc<dyn SectionStore>) -> SectionContextBuilder {
        SectionContextBuilder {
            config,
            store,
            custom_labels: BTreeMap::new(),
        }
    }

    /// The frozen `record_section` tool definition (spec §7.1.3).
    pub fn record_section_definition(&self) -> &ToolDefinition {
        &self.tool_definition
    }

    // —— 范式自身 API（特有操作 / 恢复 / 查询；全部仅空闲）——

    pub fn relabel(&mut self, section_id: u64, label: SectionLabel) -> Result<(), ContextError> {
        self.machine.require_idle("relabel")?;
        let index = self.section_index(section_id)?;
        if self.in_cache_zone(index) {
            return Err(ContextError::InvalidOperation {
                message: "cache-zone sections cannot be relabeled".to_owned(),
            });
        }
        self.sections[index].label = label;
        self.save_state_if_session()
    }

    pub fn merge_sections(
        &mut self,
        ids: Vec<u64>,
        new_label: Option<SectionLabel>,
    ) -> Result<(), ContextError> {
        self.machine.require_idle("merge_sections")?;
        if ids.len() < 2 {
            return Err(ContextError::InvalidOperation {
                message: "merge requires at least two section ids".to_owned(),
            });
        }
        let mut indices: Vec<usize> = Vec::with_capacity(ids.len());
        let mut seen = BTreeSet::new();
        for id in ids {
            if !seen.insert(id) {
                return Err(ContextError::InvalidOperation {
                    message: format!("duplicate section id {id} in merge"),
                });
            }
            let index = self.section_index(id)?;
            if self.in_cache_zone(index) {
                return Err(ContextError::InvalidOperation {
                    message: "cache-zone sections cannot be merged".to_owned(),
                });
            }
            if self.sections[index].view != View::Raw {
                return Err(ContextError::InvalidOperation {
                    message: format!("cannot merge compressed section {id}; decompress it first"),
                });
            }
            indices.push(index);
        }
        indices.sort_unstable();
        let first_label = self.sections[indices[0]].label.clone();
        let mut turns = Vec::new();
        for &index in &indices {
            turns.append(&mut self.sections[index].turns);
        }
        let label = new_label.unwrap_or(first_label);
        let merged = Section {
            id: self.next_id(),
            label,
            view: View::Raw,
            turns,
            version: 0,
            original_ref: None,
            compressed_ref: None,
            summary: None,
        };
        for &index in indices.iter().rev() {
            self.sections.remove(index);
        }
        self.sections.push(merged);
        self.save_state_if_session()
    }

    pub fn split_section(&mut self, id: u64, boundary_turn: u64) -> Result<(), ContextError> {
        self.machine.require_idle("split_section")?;
        let index = self.section_index(id)?;
        if self.in_cache_zone(index) {
            return Err(ContextError::InvalidOperation {
                message: "cache-zone sections cannot be split".to_owned(),
            });
        }
        if self.sections[index].view != View::Raw {
            return Err(ContextError::InvalidOperation {
                message: "cannot split a compressed section".to_owned(),
            });
        }
        let turn_count = self.sections[index].turns.len();
        if turn_count < 2 {
            return Err(ContextError::InvalidOperation {
                message: "cannot split a section with fewer than two turns".to_owned(),
            });
        }
        let boundary = (boundary_turn as usize).clamp(1, turn_count - 1);
        let tail = self.sections[index].turns.split_off(boundary);
        let label = self.sections[index].label.clone();
        let split = Section {
            id: self.next_id(),
            label,
            view: View::Raw,
            turns: tail,
            version: 0,
            original_ref: None,
            compressed_ref: None,
            summary: None,
        };
        self.sections.push(split);
        self.save_state_if_session()
    }

    /// Restore a compressed view from the stored snapshot without LLM calls
    /// (spec §7.1.6).
    pub fn recompress(&mut self, section_id: u64) -> Result<(), ContextError> {
        self.machine.require_idle("recompress")?;
        let index = self.section_index(section_id)?;
        if self.sections[index].view == View::Compressed {
            return Err(ContextError::InvalidOperation {
                message: "section is already compressed".to_owned(),
            });
        }
        let compressed_ref = self.sections[index].compressed_ref.clone().ok_or_else(|| {
            ContextError::InvalidOperation {
                message: "no compressed snapshot to restore".to_owned(),
            }
        })?;
        let session = self.session()?;
        let entry = self
            .store
            .load_compressed(session, &compressed_ref)
            .map_err(ContextError::from)?
            .ok_or_else(|| ContextError::InvalidOperation {
                message: "compressed snapshot is missing".to_owned(),
            })?;
        let section = &mut self.sections[index];
        section.view = View::Compressed;
        section.turns.clear();
        section.summary = Some(entry.compressed_text);
        section.version += 1;
        self.save_state_if_session()
    }

    /// Decompress by loading the stored original and replacing the view
    /// (spec §6.3). Original messages are restored as a single turn.
    pub fn decompress(&mut self, section_id: u64) -> Result<(), ContextError> {
        self.machine.require_idle("decompress")?;
        let index = self.section_index(section_id)?;
        if self.sections[index].view != View::Compressed {
            return Err(ContextError::InvalidOperation {
                message: "section is not compressed".to_owned(),
            });
        }
        let original_ref = self.sections[index].original_ref.clone().ok_or_else(|| {
            ContextError::InvalidOperation {
                message: "no stored original to restore".to_owned(),
            }
        })?;
        let session = self.session()?;
        let entry = self
            .store
            .load_original(session, &original_ref)
            .map_err(ContextError::from)?
            .ok_or_else(|| ContextError::InvalidOperation {
                message: "stored original is missing".to_owned(),
            })?;
        let section = &mut self.sections[index];
        section.view = View::Raw;
        section.turns = vec![Turn {
            messages: entry.messages,
        }];
        section.summary = None;
        section.version += 1;
        self.save_state_if_session()
    }

    pub fn section_mapping(&self, section_id: u64) -> Option<MappingRecord> {
        self.sections
            .iter()
            .find(|section| section.id == section_id)
            .map(mapping_record)
    }

    pub fn section_mappings(&self) -> Vec<MappingRecord> {
        self.sections.iter().map(mapping_record).collect()
    }

    /// Restore (or create) a session from the store (spec §7.1.6). Frozen
    /// compression pipeline states do not survive sessions; the machine is
    /// reset to `Idle` and compression is re-evaluated by the downstream.
    pub fn restore_session(&mut self, session_id: &str) -> Result<(), ContextError> {
        let state = self
            .store
            .load_state(session_id)
            .map_err(ContextError::from)?;
        match state {
            Some(state) => {
                if state.schema_version != SECTION_STATE_SCHEMA_VERSION {
                    return Err(ContextError::InvalidConfiguration {
                        message: format!(
                            "unsupported SectionState schema version {}",
                            state.schema_version
                        ),
                    });
                }
                self.sections = state.sections;
                self.token_facts = state.token_facts;
                self.next_section_id = self
                    .sections
                    .iter()
                    .map(|section| section.id)
                    .max()
                    .map_or(0, |max| max + 1);
                self.machine = CompressionMachine::new();
                for section in &mut self.sections {
                    if section.view != View::Compressed {
                        continue;
                    }
                    let compressed_ref = match section.compressed_ref.clone() {
                        Some(reference) => reference,
                        None => continue,
                    };
                    match self
                        .store
                        .load_compressed(session_id, &compressed_ref)
                        .map_err(ContextError::from)?
                    {
                        Some(entry) => section.summary = Some(entry.compressed_text),
                        None => {
                            // 压缩快照缺失 → 降级为原文视图
                            let Some(original_ref) = section.original_ref.clone() else {
                                continue;
                            };
                            let Some(original) = self
                                .store
                                .load_original(session_id, &original_ref)
                                .map_err(ContextError::from)?
                            else {
                                continue;
                            };
                            section.view = View::Raw;
                            section.turns = vec![Turn {
                                messages: original.messages,
                            }];
                            section.summary = None;
                        }
                    }
                }
                self.session_id = Some(session_id.to_owned());
            }
            None => {
                self.session_id = Some(session_id.to_owned());
                self.sections.clear();
                self.next_section_id = 0;
                self.token_facts = TokenFacts::default();
                self.machine = CompressionMachine::new();
            }
        }
        Ok(())
    }

    pub fn compression_state(&self) -> CompressionState {
        self.machine.state()
    }

    // —— 内部辅助 ——

    fn section_index(&self, section_id: u64) -> Result<usize, ContextError> {
        self.sections
            .iter()
            .position(|section| section.id == section_id)
            .ok_or_else(|| ContextError::InvalidOperation {
                message: format!("section {section_id} does not exist"),
            })
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_section_id;
        self.next_section_id += 1;
        id
    }

    fn session(&self) -> Result<&str, ContextError> {
        self.session_id
            .as_deref()
            .ok_or_else(|| ContextError::InvalidOperation {
                message: "restore_session must be called before persistent operations".to_owned(),
            })
    }

    fn in_cache_zone(&self, index: usize) -> bool {
        index < self.config.cache_prefix_sections
    }

    /// First index of the active zone; the newest section is always active as
    /// the write target (spec §7.1.2).
    fn active_start_index(&self) -> usize {
        let cache = self.config.cache_prefix_sections.min(self.sections.len());
        match self.config.active_window {
            ActiveWindow::Sections { count: window } => {
                let window = window.max(1);
                self.sections.len().saturating_sub(window).max(cache)
            }
            ActiveWindow::All => cache,
            ActiveWindow::Hyper => self.sections.len().saturating_sub(1).max(cache),
        }
    }

    fn compute_window_state(&self) -> WindowState {
        let cache = self.config.cache_prefix_sections.min(self.sections.len());
        let active_start = self.active_start_index();
        WindowState {
            mode: self.config.active_window,
            cache_prefix_sections: cache,
            sealed_count: active_start.saturating_sub(cache),
            active_count: self.sections.len().saturating_sub(active_start),
        }
    }

    fn policy_for(&self, label: &SectionLabel) -> LabelPolicy {
        let fallback = LabelPolicy {
            compressible: false,
            priority: 0,
            method: CompressionMethod::Shallow,
        };
        match label {
            SectionLabel::Standard(standard) => self
                .config
                .label_policies
                .get(standard)
                .copied()
                .unwrap_or(fallback),
            SectionLabel::Custom(name) => self
                .custom_label_policies
                .get(name)
                .copied()
                .unwrap_or(fallback),
        }
    }

    fn is_compressible_label(&self, label: &SectionLabel) -> bool {
        self.policy_for(label).compressible
    }

    fn save_state_if_session(&self) -> Result<(), ContextError> {
        if let Some(session) = self.session_id.as_deref() {
            let state = SectionState {
                schema_version: SECTION_STATE_SCHEMA_VERSION,
                session_id: session.to_owned(),
                sections: self.sections.clone(),
                window: self.compute_window_state(),
                machine: self.machine.state(),
                token_facts: self.token_facts,
            };
            self.store.save_state(&state).map_err(ContextError::from)?;
        }
        Ok(())
    }

    /// Ensure a Raw, non-cache write target exists; the newest section is the
    /// write target unless it is cache-zone (the cache prefix is frozen) or
    /// compressed (defensive), in which case a new section is created.
    fn ensure_write_target(&mut self) {
        let needs_section = if self.sections.is_empty() {
            true
        } else {
            let last = self.sections.len() - 1;
            self.in_cache_zone(last) || self.sections[last].view != View::Raw
        };
        if needs_section {
            let id = self.next_id();
            self.sections.push(Section::new(
                id,
                SectionLabel::Standard(StandardLabel::Uncategorized),
            ));
        }
    }

    fn append_message(&mut self, message: Message) {
        self.ensure_write_target();
        let last = self.sections.len() - 1;
        let section = &mut self.sections[last];
        if let Some(turn) = section.turns.last_mut() {
            turn.messages.push(message);
        } else {
            section.turns.push(Turn {
                messages: vec![message],
            });
        }
    }

    fn start_new_turn(&mut self, message: Message) {
        self.ensure_write_target();
        let last = self.sections.len() - 1;
        self.sections[last].turns.push(Turn {
            messages: vec![message],
        });
    }

    /// `record_section` division (spec §7.1.4). Returns whether a new section
    /// was carved.
    fn divide_sections(
        &mut self,
        rounds: u64,
        label: Option<SectionLabel>,
    ) -> Result<bool, ContextError> {
        let total_turns: usize = self
            .sections
            .iter()
            .map(|section| section.turns.len())
            .sum();
        if total_turns == 0 {
            return Ok(false);
        }
        let rounds = (rounds as usize).clamp(1, total_turns);

        // 从最新小节往回收集 rounds 个轮次；只收集活跃区（Open）部分的轮次
        let active_start = self.active_start_index();
        let mut collected: Vec<(usize, usize)> = Vec::with_capacity(rounds);
        let mut remaining = rounds;
        'collect: for section_index in (0..self.sections.len()).rev() {
            if section_index < active_start {
                break; // 触及 Sealed/缓存区 → 只合并 Open 部分
            }
            let turns = self.sections[section_index].turns.len();
            let take = turns.min(remaining);
            for turn_index in (turns - take)..turns {
                collected.push((section_index, turn_index));
            }
            remaining -= take;
            if remaining == 0 {
                break 'collect;
            }
        }
        if collected.is_empty() {
            return Ok(false);
        }

        let newest = self.sections.len() - 1;
        let all_in_newest = collected
            .iter()
            .all(|(section_index, _)| *section_index == newest);
        let fills_newest = collected.len() == self.sections[newest].turns.len();
        if all_in_newest && fills_newest {
            // 幂等：边界已一致，仅应用标签
            if let Some(label) = label {
                self.sections[newest].label = label;
            }
            return Ok(false);
        }

        // 跨小节（含子集）→ 裁出新小节（轮次恢复为时间顺序）
        let source_label = self.sections[collected[0].0].label.clone();
        let mut new_turns: Vec<Turn> = collected
            .iter()
            .map(|(section_index, turn_index)| {
                self.sections[*section_index].turns[*turn_index].clone()
            })
            .collect();
        new_turns.reverse();
        let mut by_section: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (section_index, turn_index) in &collected {
            by_section
                .entry(*section_index)
                .or_default()
                .push(*turn_index);
        }
        for (section_index, turn_indices) in by_section {
            let section = &mut self.sections[section_index];
            for turn_index in turn_indices.into_iter().rev() {
                section.turns.remove(turn_index);
            }
        }
        // 清理空小节（从后往前；缓存区不会被触及）
        let mut index = self.sections.len();
        while index > 0 {
            index -= 1;
            if self.sections[index].turns.is_empty() && !self.in_cache_zone(index) {
                self.sections.remove(index);
            }
        }
        let new_label = label.unwrap_or(source_label);
        let id = self.next_id();
        self.sections.push(Section {
            id,
            label: new_label,
            view: View::Raw,
            turns: new_turns,
            version: 0,
            original_ref: None,
            compressed_ref: None,
            summary: None,
        });
        Ok(true)
    }
}

// —— Context 实现 ——

impl Context for SectionContext {
    fn push_user_input(&mut self, message: Message) -> Result<(), ContextError> {
        self.machine.require_idle("push_user_input")?;
        self.start_new_turn(message);
        self.last_event_carved_section = false;
        self.save_state_if_session()
    }

    fn apply_model_output(
        &mut self,
        message: Message,
        usage: TokenUsage,
    ) -> Result<(), ContextError> {
        self.machine.require_idle("apply_model_output")?;
        if let Some(input) = usage.input_tokens {
            self.token_facts.input_tokens = input;
        }
        let call = find_record_section_call(&message);
        self.append_message(message);
        let carved = match call {
            Some((rounds, label)) => {
                let label = label_from_arguments(label, self);
                self.divide_sections(rounds, label)?
            }
            None => false,
        };
        self.last_event_carved_section = carved;
        self.save_state_if_session()
    }

    fn export(&self) -> Result<Vec<Message>, ContextError> {
        let cache = self.config.cache_prefix_sections.min(self.sections.len());
        let active_start = self.active_start_index();
        let mut out = Vec::new();
        for (index, section) in self.sections.iter().enumerate() {
            let sealed = index >= cache && index < active_start;
            match section.view {
                View::Raw => out.extend(flatten_turns(&section.turns)),
                View::Compressed => {
                    if sealed || index >= active_start {
                        let summary = section.summary.as_ref().ok_or_else(|| {
                            ContextError::InvalidRequest {
                                message: "compressed section has no loaded summary".to_owned(),
                            }
                        })?;
                        for message in summary {
                            let mut message = message.clone();
                            message.role = self.config.compressed_message_role;
                            out.push(message);
                        }
                    }
                }
            }
        }
        let stripped = strip_record_section_traces(out);
        if stripped.is_empty() {
            return Err(ContextError::InvalidRequest {
                message: "exported message sequence is empty".to_owned(),
            });
        }
        validate_convert_contract(&stripped)?;
        Ok(stripped)
    }

    fn evaluate_compression(&mut self) -> Result<Option<CompressionTarget>, ContextError> {
        let target = self.compute_compression_target();
        self.machine.on_evaluate(target.clone())?;
        Ok(target)
    }

    fn prepare_compression(
        &mut self,
        target: CompressionTarget,
    ) -> Result<Vec<Message>, ContextError> {
        self.machine.on_prepare(&target)?;
        let section_id = match target {
            CompressionTarget::Section { id } => id,
        };
        let index = self.section_index(section_id)?;
        if self.in_cache_zone(index) {
            return Err(ContextError::InvalidOperation {
                message: "cache-zone sections are never compressed".to_owned(),
            });
        }
        let section = &self.sections[index];
        if section.view != View::Raw {
            return Err(ContextError::InvalidOperation {
                message: "section is already compressed".to_owned(),
            });
        }
        if !self.is_compressible_label(&section.label) {
            return Err(ContextError::InvalidOperation {
                message: "section label is not compressible".to_owned(),
            });
        }
        if matches!(self.config.tool_turn_policy, ToolTurnPolicy::Reject)
            && section_has_tool_turns(section)
        {
            return Err(ContextError::InvalidOperation {
                message: "tool turn policy rejects sections with tool turns".to_owned(),
            });
        }

        // 原文先落盘
        let snapshot = OriginalSnapshot {
            section_id,
            messages: flatten_turns(&section.turns),
            version: section.version,
        };
        let original_ref = self.persist_original(snapshot)?;
        self.pending_original = Some((section_id, original_ref.clone()));

        // 指令消息 + 目标内容（下游零组装）
        let instruction =
            self.compression_instruction(&section.label, section_has_tool_turns(section));
        // 目标内容必须"可直接推理"：剥离 record_section 簿记痕迹（与 export 同规则），
        // 否则未配对的 tool_calls 会被严格 Provider 拒绝（Live 验证发现）。
        let target = strip_record_section_traces(flatten_turns(&section.turns));
        let mut messages = vec![instruction];
        messages.extend(target);
        Ok(messages)
    }

    fn apply_compression_result(&mut self, summary: Vec<Message>) -> Result<(), ContextError> {
        self.machine.on_apply()?;
        let (section_id, original_ref) =
            self.pending_original
                .take()
                .ok_or_else(|| ContextError::InvalidOperation {
                    message: "no prepared original to commit".to_owned(),
                })?;
        let index = self.section_index(section_id)?;
        let version = self.sections[index].version + 1;
        let session = self.session()?.to_owned();

        // 压缩条目（summary 原生承载，无 JSON 序列化）
        let record_id = format!("compressed-{section_id}-{version}");
        let entry = SectionCompressedEntry {
            session_id: session.clone(),
            record_id,
            compressed_text: summary.clone(),
            original_ref: original_ref.clone(),
            version,
            archived_at: SystemTime::now(),
        };
        let compressed_ref = self
            .store
            .save_compressed(&entry)
            .map_err(ContextError::from)?;

        let section = &mut self.sections[index];
        section.view = View::Compressed;
        section.turns.clear();
        section.version = version;
        section.original_ref = Some(original_ref);
        section.compressed_ref = Some(compressed_ref);
        section.summary = Some(summary);
        self.save_state_if_session()
    }

    fn abandon_compression(&mut self) -> Result<(), ContextError> {
        self.machine.on_abandon()?;
        if let (Some((_section_id, original_ref)), Some(session)) =
            (self.pending_original.take(), self.session_id.as_deref())
        {
            self.store
                .delete_original(session, &original_ref)
                .map_err(ContextError::from)?;
        }
        Ok(())
    }
}

// —— 压缩触发与指令 ——

impl SectionContext {
    fn compute_compression_target(&self) -> Option<CompressionTarget> {
        if let ActiveWindow::Hyper = self.config.active_window {
            if !self.last_event_carved_section {
                return None;
            }
            // 活跃窗口 0：刚结束小节立即压缩（无条件触发，仍受硬约束）
            let section = self.sections.last()?;
            if self.is_compressible_candidate(section) {
                return Some(CompressionTarget::Section { id: section.id });
            }
            return None;
        }
        let triggered = match self.config.auto_compression {
            Some(AutoCompression::TokenThreshold { threshold }) => {
                self.token_facts.input_tokens >= threshold
            }
            Some(AutoCompression::SectionSwitch) => self.last_event_carved_section,
            None => false,
        };
        if !triggered {
            return None;
        }
        self.best_sealed_candidate()
            .map(|section| CompressionTarget::Section { id: section.id })
    }

    /// 压缩候选硬约束：固化区 ∩ Raw ∩ 可压缩标签；`Reject` 策略下含工具轮次的小节排除
    /// （Spec §7.1.0：评估阶段即排除）。
    fn is_compressible_candidate(&self, section: &Section) -> bool {
        if section.view != View::Raw || !self.is_compressible_label(&section.label) {
            return false;
        }
        if matches!(self.config.tool_turn_policy, ToolTurnPolicy::Reject)
            && section_has_tool_turns(section)
        {
            return false;
        }
        true
    }

    fn best_sealed_candidate(&self) -> Option<&Section> {
        let cache = self.config.cache_prefix_sections.min(self.sections.len());
        let active_start = self.active_start_index();
        self.sections
            .iter()
            .enumerate()
            .filter(|(index, _)| *index >= cache && *index < active_start)
            .filter(|(_, section)| self.is_compressible_candidate(section))
            .max_by_key(|(_, section)| {
                (
                    self.policy_for(&section.label).priority,
                    Reverse(section.id),
                )
            })
            .map(|(_, section)| section)
    }

    fn persist_original(&self, snapshot: OriginalSnapshot) -> Result<OriginalRef, ContextError> {
        let session = self.session()?.to_owned();
        let entry = SectionOriginalEntry {
            session_id: session.clone(),
            target: CompressionTarget::Section {
                id: snapshot.section_id,
            },
            messages: snapshot.messages,
            version: snapshot.version,
            archived_at: SystemTime::now(),
        };
        self.store.save_original(&entry).map_err(ContextError::from)
    }

    fn compression_instruction(&self, label: &SectionLabel, has_tool_turns: bool) -> Message {
        let policy = self.policy_for(label);
        let depth = match policy.method {
            CompressionMethod::Shallow => "浅压缩（保留结构）",
            CompressionMethod::Deep => "深压缩（精简细节）",
        };
        let tool = if has_tool_turns {
            match self.config.tool_turn_policy {
                ToolTurnPolicy::Downgrade => "；工具轮次降级为自然语言摘要",
                ToolTurnPolicy::Reject => "（本小节不含工具轮次）",
            }
        } else {
            ""
        };
        let tokens = self
            .config
            .compression_token_target
            .map(|target| target.to_string())
            .unwrap_or_else(|| "范式默认".to_owned());
        let text = format!(
            "请将以下对话小节压缩为简洁摘要，保持小节结构与关键事实。压缩深度：{depth}{tool}。目标输出 token 数：{tokens}。"
        );
        Message::new(Role::System, vec![ContentPart::text(text)])
    }
}

// —— 独立辅助函数 ——

fn flatten_turns(turns: &[Turn]) -> Vec<Message> {
    let mut messages = Vec::new();
    for turn in turns {
        messages.extend(turn.messages.iter().cloned());
    }
    messages
}

fn section_has_tool_turns(section: &Section) -> bool {
    section
        .turns
        .iter()
        .flat_map(|turn| &turn.messages)
        .any(|message| {
            message
                .content
                .iter()
                .any(|part| matches!(part, ContentPart::ToolCall(_)))
        })
}

fn find_record_section_call(message: &Message) -> Option<(u64, Option<String>)> {
    for part in &message.content {
        if let ContentPart::ToolCall(call) = part
            && call.name == RECORD_SECTION_TOOL_NAME
        {
            let rounds = call
                .arguments
                .get("section_start_rounds")
                .and_then(Value::as_u64)
                .unwrap_or(1);
            let label = call
                .arguments
                .get("label")
                .and_then(Value::as_str)
                .map(str::to_owned);
            return Some((rounds, label));
        }
    }
    None
}

fn label_from_arguments(label: Option<String>, context: &SectionContext) -> Option<SectionLabel> {
    let name = label?;
    if let Some(standard) = StandardLabel::from_wire_name(&name) {
        return Some(SectionLabel::Standard(standard));
    }
    if context.custom_label_policies.contains_key(&name) {
        return Some(SectionLabel::Custom(name));
    }
    None
}

/// Strip `record_section` tool calls and their matching results (spec §8.1).
fn strip_record_section_traces(messages: Vec<Message>) -> Vec<Message> {
    let mut call_ids: BTreeSet<String> = BTreeSet::new();
    for message in &messages {
        if message.role != Role::Assistant {
            continue;
        }
        for part in &message.content {
            if let ContentPart::ToolCall(call) = part
                && call.name == RECORD_SECTION_TOOL_NAME
            {
                call_ids.insert(call.id.as_str().to_owned());
            }
        }
    }
    let mut out = Vec::with_capacity(messages.len());
    for message in messages {
        let content: Vec<ContentPart> = message
            .content
            .into_iter()
            .filter(|part| match part {
                ContentPart::ToolCall(call) => call.name != RECORD_SECTION_TOOL_NAME,
                ContentPart::ToolResult(result) => !call_ids.contains(result.call_id.as_str()),
                _ => true,
            })
            .collect();
        if !content.is_empty() {
            out.push(Message {
                role: message.role,
                content,
            });
        }
    }
    out
}

/// convert.rs contract of the export output (spec §8.3).
fn validate_convert_contract(messages: &[Message]) -> Result<(), ContextError> {
    for message in messages {
        if message.content.is_empty() {
            return Err(ContextError::InvalidRequest {
                message: "exported message content must not be empty".to_owned(),
            });
        }
        for part in &message.content {
            match (message.role, part) {
                (Role::System, ContentPart::Text(_)) => {}
                (Role::System, _) => {
                    return Err(ContextError::InvalidRequest {
                        message: "system messages must contain text only".to_owned(),
                    });
                }
                (Role::User, ContentPart::ToolCall(_)) => {
                    return Err(ContextError::InvalidRequest {
                        message: "user messages must not contain tool calls".to_owned(),
                    });
                }
                (Role::Assistant, ContentPart::ToolResult(_)) => {
                    return Err(ContextError::InvalidRequest {
                        message: "assistant messages must not contain tool results".to_owned(),
                    });
                }
                (Role::Tool, ContentPart::ToolResult(_)) => {}
                (Role::Tool, _) => {
                    return Err(ContextError::InvalidRequest {
                        message: "tool messages must contain tool results only".to_owned(),
                    });
                }
                (_, ContentPart::ProviderData(_)) => {
                    return Err(ContextError::InvalidRequest {
                        message: "exported messages must not contain provider data".to_owned(),
                    });
                }
                _ => {}
            }
        }
    }
    Ok(())
}
