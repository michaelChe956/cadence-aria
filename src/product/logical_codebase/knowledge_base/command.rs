//! 知识库 provider turn 的命令文本：provider 前缀 + 固定命令名。
//!
//! 前缀唯一来源 `RepositoryInitializationStepKind::skill_command_prefix`
//! （Claude `/`、Pi `/skill:`、Codex `$cadence-init:`），本模块不另行拼装
//! 其它形态；provider 无前缀（Fake）时 fail-closed 返回 `None`。

use crate::product::models::ProviderName;
use crate::product::repository_store::RepositoryInitializationStepKind;

const BOOTSTRAP_COMMAND_NAME: &str = "knowledge-base-bootstrap";
const UPDATE_COMMAND_NAME: &str = "knowledge-base-update";
const USER_INPUT_COMMAND_NAME: &str = "knowledge-base-user-input";

/// bootstrap 命令：`<前缀>knowledge-base-bootstrap`。
pub fn bootstrap_command(provider: &ProviderName) -> Option<String> {
    let prefix = RepositoryInitializationStepKind::skill_command_prefix(provider.clone())?;
    Some(format!("{prefix}{BOOTSTRAP_COMMAND_NAME}"))
}

/// update 命令：`<前缀>knowledge-base-update <变更包目录>`。
///
/// `package_rel_path` 是相对聚合根的变更包目录，作为命令的**单个参数**
/// 注入；为空、含空白或控制字符时无法保证单参数语义，fail-closed 返回
/// `None`（路径合法性本身由 `change_package` 的机械门禁负责）。
pub fn update_command(provider: &ProviderName, package_rel_path: &str) -> Option<String> {
    if package_rel_path.is_empty()
        || package_rel_path
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return None;
    }
    let prefix = RepositoryInitializationStepKind::skill_command_prefix(provider.clone())?;
    Some(format!("{prefix}{UPDATE_COMMAND_NAME} {package_rel_path}"))
}

/// user_input 起草命令：`<前缀>knowledge-base-user-input`（无参数）。
///
/// 起草 turn 在 canonical 聚合根执行；skill 契约只写
/// `cadence/knowledge-base/user-input/`（写入面审计据此收紧允许面）。
pub fn user_input_command(provider: &ProviderName) -> Option<String> {
    let prefix = RepositoryInitializationStepKind::skill_command_prefix(provider.clone())?;
    Some(format!("{prefix}{USER_INPUT_COMMAND_NAME}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACKAGE: &str = "cadence/knowledge-base/user-input/updates/CHANGE-x";

    #[test]
    fn bootstrap_command_uses_provider_skill_prefix() {
        assert_eq!(
            bootstrap_command(&ProviderName::ClaudeCode).as_deref(),
            Some("/knowledge-base-bootstrap")
        );
        assert_eq!(
            bootstrap_command(&ProviderName::Pi).as_deref(),
            Some("/skill:knowledge-base-bootstrap")
        );
        assert_eq!(
            bootstrap_command(&ProviderName::Codex).as_deref(),
            Some("$cadence-init:knowledge-base-bootstrap")
        );
        assert_eq!(bootstrap_command(&ProviderName::Fake), None);
    }

    #[test]
    fn update_command_appends_package_path() {
        assert_eq!(
            update_command(&ProviderName::Codex, PACKAGE).as_deref(),
            Some(
                "$cadence-init:knowledge-base-update cadence/knowledge-base/user-input/updates/CHANGE-x"
            )
        );
        assert_eq!(
            update_command(&ProviderName::ClaudeCode, PACKAGE).as_deref(),
            Some("/knowledge-base-update cadence/knowledge-base/user-input/updates/CHANGE-x")
        );
        assert_eq!(
            update_command(&ProviderName::Pi, PACKAGE).as_deref(),
            Some("/skill:knowledge-base-update cadence/knowledge-base/user-input/updates/CHANGE-x")
        );
        assert_eq!(update_command(&ProviderName::Fake, PACKAGE), None);
    }

    #[test]
    fn update_command_rejects_package_path_that_is_not_a_single_argument() {
        for bad in ["", "a b", "a\nb", "a\tb", "a\u{7}b"] {
            assert_eq!(
                update_command(&ProviderName::ClaudeCode, bad),
                None,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn user_input_command_uses_provider_skill_prefix() {
        assert_eq!(
            user_input_command(&ProviderName::ClaudeCode).as_deref(),
            Some("/knowledge-base-user-input")
        );
        assert_eq!(
            user_input_command(&ProviderName::Pi).as_deref(),
            Some("/skill:knowledge-base-user-input")
        );
        assert_eq!(
            user_input_command(&ProviderName::Codex).as_deref(),
            Some("$cadence-init:knowledge-base-user-input")
        );
        assert_eq!(user_input_command(&ProviderName::Fake), None);
    }
}
