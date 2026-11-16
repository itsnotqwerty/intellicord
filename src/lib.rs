use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

pub const HISTORY_LIMIT: usize = 500;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct User {
    pub id: String,
    pub username: String,
    pub display_name: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Channel {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Command {
    Snapshot {
        channel: Option<Channel>,
        users: Vec<User>,
    },
    Clear,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Joined,
    Left,
}

#[derive(Debug, Serialize)]
pub struct Event {
    pub sequence: u64,
    pub timestamp: u64,
    pub action: Action,
    pub user: User,
    pub channel: Channel,
}

#[derive(Default, Debug, Serialize)]
pub struct History {
    pub channel: Option<Channel>,
    pub participants: BTreeMap<String, User>,
    pub events: VecDeque<Event>,
    #[serde(skip)]
    sequence: u64,
}

impl History {
    pub fn apply(&mut self, command: Command, timestamp: u64) -> Result<(), &'static str> {
        match command {
            Command::Clear => self.events.clear(),
            Command::Snapshot { channel, users } => {
                if channel.as_ref().is_some_and(|value| !valid_id(&value.id))
                    || users.iter().any(|user| !valid_id(&user.id))
                    || users.len() > 10_000
                {
                    return Err("invalid snapshot");
                }
                let participants: BTreeMap<_, _> = users
                    .into_iter()
                    .map(|user| (user.id.clone(), user))
                    .collect();
                let same_channel = self.channel.as_ref().map(|value| &value.id)
                    == channel.as_ref().map(|value| &value.id);
                if same_channel && let Some(channel) = &channel {
                    let mut changes = Vec::new();
                    for (id, user) in &self.participants {
                        if !participants.contains_key(id) {
                            changes.push((Action::Left, user.clone()));
                        }
                    }
                    for (id, user) in &participants {
                        if !self.participants.contains_key(id) {
                            changes.push((Action::Joined, user.clone()));
                        }
                    }
                    for (action, user) in changes {
                        self.sequence += 1;
                        self.events.push_back(Event {
                            sequence: self.sequence,
                            timestamp,
                            action,
                            user,
                            channel: channel.clone(),
                        });
                        if self.events.len() > HISTORY_LIMIT {
                            self.events.pop_front();
                        }
                    }
                }
                self.participants = if channel.is_some() {
                    participants
                } else {
                    BTreeMap::new()
                };
                self.channel = channel;
            }
        }
        Ok(())
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 20 && id.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(channel: Option<&str>, ids: &[&str]) -> Command {
        Command::Snapshot {
            channel: channel.map(|id| Channel {
                id: id.into(),
                name: "Lounge".into(),
            }),
            users: ids
                .iter()
                .map(|id| User {
                    id: (*id).into(),
                    username: format!("user_{id}"),
                    display_name: format!("User {id}"),
                    avatar_url: None,
                })
                .collect(),
        }
    }

    #[test]
    fn avatar_metadata_survives_history_without_creating_membership_events() {
        let command = |ids: &[&str], avatar: &str| {
            let mut value = snapshot(Some("100"), ids);
            if let Command::Snapshot { users, .. } = &mut value {
                for user in users {
                    user.avatar_url = Some(avatar.into());
                }
            }
            value
        };
        let mut history = History::default();
        history
            .apply(
                command(&["1"], "https://cdn.discordapp.com/avatars/1/first.png"),
                1,
            )
            .unwrap();
        history
            .apply(
                command(&["1"], "https://cdn.discordapp.com/avatars/1/updated.png"),
                2,
            )
            .unwrap();
        assert!(history.events.is_empty());
        history.apply(snapshot(Some("100"), &[]), 3).unwrap();
        assert_eq!(
            history.events[0].user.avatar_url.as_deref(),
            Some("https://cdn.discordapp.com/avatars/1/updated.png")
        );
        let legacy: User =
            serde_json::from_str(r#"{"id":"1","username":"one","display_name":"One"}"#).unwrap();
        assert_eq!(legacy.avatar_url, None);
        assert_eq!(
            serde_json::to_value(&history).unwrap()["events"][0]["user"]["avatar_url"],
            "https://cdn.discordapp.com/avatars/1/updated.png"
        );
    }

    #[test]
    fn baseline_then_join_leave_and_duplicate() {
        let mut history = History::default();
        history
            .apply(snapshot(Some("100"), &["9007199254740993"]), 1)
            .unwrap();
        assert!(history.events.is_empty());
        history
            .apply(snapshot(Some("100"), &["9007199254740993", "2"]), 2)
            .unwrap();
        history.apply(snapshot(Some("100"), &["2"]), 3).unwrap();
        history.apply(snapshot(Some("100"), &["2"]), 4).unwrap();
        assert_eq!(history.events.len(), 2);
        assert_eq!(history.events[0].action, Action::Joined);
        assert_eq!(history.events[1].action, Action::Left);
        assert_eq!(history.events[1].user.id, "9007199254740993");
        assert_eq!(history.events[1].timestamp, 3);
    }

    #[test]
    fn channel_switch_and_disconnect_do_not_invent_events() {
        let mut history = History::default();
        for command in [
            snapshot(Some("100"), &["1"]),
            snapshot(Some("200"), &["2"]),
            snapshot(None, &[]),
            snapshot(Some("200"), &["3"]),
        ] {
            history.apply(command, 1).unwrap();
        }
        assert!(history.events.is_empty());
        assert!(history.participants.contains_key("3"));
    }

    #[test]
    fn clear_keeps_baseline_and_history_is_bounded() {
        let mut history = History::default();
        history.apply(snapshot(Some("100"), &[]), 0).unwrap();
        for timestamp in 1..=600 {
            let ids = if timestamp % 2 == 0 {
                vec![]
            } else {
                vec!["1"]
            };
            history
                .apply(snapshot(Some("100"), &ids), timestamp)
                .unwrap();
        }
        assert_eq!(history.events.len(), HISTORY_LIMIT);
        assert_eq!(history.events[0].sequence, 101);
        history.apply(Command::Clear, 601).unwrap();
        history.apply(snapshot(Some("100"), &[]), 602).unwrap();
        assert!(history.events.is_empty());
    }

    #[test]
    fn bad_snapshot_is_atomic_and_names_do_not_trigger_events() {
        let mut history = History::default();
        history.apply(snapshot(Some("100"), &["1"]), 0).unwrap();
        assert!(
            history
                .apply(snapshot(Some("100"), &["not-an-id"]), 1)
                .is_err()
        );
        assert!(history.participants.contains_key("1"));
        let Command::Snapshot { channel, mut users } = snapshot(Some("100"), &["1"]) else {
            panic!()
        };
        users[0].username = "renamed".into();
        history
            .apply(Command::Snapshot { channel, users }, 2)
            .unwrap();
        assert!(history.events.is_empty());
        assert_eq!(history.participants["1"].username, "renamed");
    }
}
