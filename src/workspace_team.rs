//! Client projection of server-owned workspace peers (OUP workspace-team v1).
use crate::menu::{LocalAction, MenuAction, MenuBuildResult, MenuId, MenuItem, MenuMode, MenuSpec};
use octos_core::SessionKey;
use rust_i18n::t;
use serde::{Deserialize, Serialize};

pub const LIST: &str = "peer/team/list";
pub const LEADER: &str = "peer/team/leader/set";
pub const MESSAGE: &str = "peer/team/message";
pub const MENU: &str = "workspace-agents";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Member {
    pub agent_id: String,
    pub session_id: SessionKey,
    pub role: String,
    pub status: String,
    pub attached: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub session_id: SessionKey,
    pub workspace: String,
    pub revision: u64,
    pub leader: String,
    pub members: Vec<Member>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessageResult {
    pub session_id: SessionKey,
    pub receipts: Vec<Receipt>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    pub agent_id: String,
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Command {
    Leader {
        session_id: SessionKey,
        agent_id: String,
        expected_revision: u64,
    },
    Message {
        session_id: SessionKey,
        agent_id: Option<String>,
        broadcast: bool,
        message: String,
        occurrence_id: String,
    },
    List {
        session_id: SessionKey,
    },
}

impl Command {
    pub fn method(&self) -> &'static str {
        match self {
            Self::List { .. } => LIST,
            Self::Leader { .. } => LEADER,
            Self::Message { .. } => MESSAGE,
        }
    }
}

pub fn occurrence() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "{:x}-{:x}-{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

pub fn menu(team: &Snapshot, readonly: bool) -> MenuBuildResult {
    let mut items = Vec::new();
    for member in &team.members {
        let you = if member.session_id == team.session_id {
            t!("workspace_team.you").into_owned()
        } else {
            String::new()
        };
        let presence = if member.attached {
            t!("workspace_team.connected").into_owned()
        } else {
            t!("workspace_team.detached").into_owned()
        };
        let role = if member.role == "coordinator" {
            t!("workspace_team.coordinator")
        } else {
            t!("workspace_team.member")
        };
        let status = if member.status == "running" {
            t!("workspace_team.running")
        } else {
            t!("workspace_team.idle")
        };
        items.push(
            MenuItem::new(
                format!("team.message.{}", member.agent_id),
                format!(
                    "{} · {} · {} · {}{}",
                    member.agent_id, role, status, presence, you
                ),
                if readonly || member.session_id == team.session_id {
                    MenuAction::Noop
                } else {
                    MenuAction::Local(LocalAction::EditComposer(format!(
                        "/agents message {} ",
                        member.agent_id
                    )))
                },
            )
            .with_description(member.session_id.0.clone()),
        );
        if !readonly && member.agent_id != team.leader {
            items.push(MenuItem::new(
                format!("team.leader.{}", member.agent_id),
                t!("workspace_team.make_coordinator", agent = member.agent_id).into_owned(),
                MenuAction::send_appui(crate::model::AppUiCommand::WorkspaceTeam(
                    Command::Leader {
                        session_id: team.session_id.clone(),
                        agent_id: member.agent_id.clone(),
                        expected_revision: team.revision,
                    },
                )),
            ));
        }
    }
    items.push(MenuItem::new(
        "team.refresh",
        t!("workspace_team.refresh").into_owned(),
        MenuAction::send_appui(crate::model::AppUiCommand::WorkspaceTeam(Command::List {
            session_id: team.session_id.clone(),
        })),
    ));
    MenuBuildResult::Ready(MenuSpec {
        id: MenuId::from(MENU),
        title: t!("workspace_team.title").into_owned(),
        subtitle: Some(team.workspace.clone()),
        items,
        tabs: Vec::new(),
        searchable: false,
        search_placeholder: None,
        footer_hint: Some(t!("workspace_team.hint").into_owned()),
        preview: None,
        mode: MenuMode::SingleSelect,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_picker_leader_action_uses_displayed_revision_and_readonly_has_no_mutations() {
        let team = Snapshot {
            session_id: SessionKey("local:a".into()),
            workspace: "/repo".into(),
            revision: 9,
            leader: "workspace-1".into(),
            members: vec![Member {
                agent_id: "workspace-2".into(),
                session_id: SessionKey("local:b".into()),
                role: "member".into(),
                status: "running".into(),
                attached: true,
            }],
        };
        let MenuBuildResult::Ready(menu) = menu(&team, false) else {
            panic!("menu")
        };
        assert!(menu.items.iter().any(|item| matches!(&item.action, MenuAction::SendAppUi(command) if matches!(command.as_ref(), crate::model::AppUiCommand::WorkspaceTeam(Command::Leader { expected_revision: 9, .. })))));
        let MenuBuildResult::Ready(readonly) = super::menu(&team, true) else {
            panic!("menu")
        };
        assert!(
            readonly
                .items
                .iter()
                .all(|item| item.id.as_str() == "team.refresh"
                    || matches!(item.action, MenuAction::Noop))
        );
    }
}
