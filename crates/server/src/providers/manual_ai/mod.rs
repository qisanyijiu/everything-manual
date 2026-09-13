//! 说明书 AI 适配器（T14；REQ-029 / AC-045、AC-046）。
//!
//! 组成：
//! - [`client`]：reqwest HTTP 客户端（超时、鉴权、**无自动重试**、不跟随重定向、
//!   响应体读取有上限、错误分类与脱敏）；
//! - [`dto`]：Responses 请求构造（`text.format.type=json_schema`、strict、
//!   `max_output_tokens` 受限、`store=false`）与响应解析（`output[].content[]` 的
//!   `output_text` / `refusal` / `incomplete` / `usage`）；
//! - [`prompt`]：`manual_extract_v1` 提示词模板（资料是待分析数据不是指令）；
//! - [`store`]：批次结果/诊断的内容寻址持久化（先结果资产、后 usage/receipt）；
//! - [`handlers`]：`manual_extract`（按 `batch_index`）与 `manual_merge` 阶段处理器，
//!   由 `serve` 按配置注册进 T10 执行器。
//!
//! 领域类型、服务端再次校验与本地确定性合并（`manual_core::knowledge`）：
//! 模型输出只是候选；拒答/截断/格式错/引用校验失败**不产生正式知识**；
//! 合并去重保留原始出处，同名不同事实保留冲突待复核。
//!
//! 真实调用边界：本卡的验证全部指向本机 fixture（`127.0.0.1`，零外网）；
//! 真实模型效果与费用只在 T23 的授权入口执行。

pub mod client;
pub mod dto;
pub mod handlers;
pub mod prompt;
pub mod store;

pub use client::{
    MANUAL_AI_RESPONSES_PATH, MAX_RESPONSE_BYTES, ManualAiClient, ManualAiError, ManualAiTimeouts,
    RawResponse,
};
pub use dto::{
    ExtractRequest, ExtractUsage, InputContent, InputMessage, ParsedResponse, jpeg_data_url,
};
pub use handlers::{
    ManualAiHandlers, ManualExtractHandler, ManualMergeHandler, stage_endpoint_summary,
};
pub use prompt::{PROMPT_TEMPLATE_VERSION, PromptPage, build_batch_prompt};
