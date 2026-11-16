use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

pub const HISTORY_LIMIT: usize = 500;
pub const ARCHIVE_LIMIT: usize = 20;

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

#[derive(Debug, Serialize)]
pub struct CallInfo {
    pub id: u64,
    pub channel: Channel,
    pub started_at: u64,
    pub ended_at: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct CallArchive {
    #[serde(flatten)]
    pub call: CallInfo,
    pub events: VecDeque<Event>,
}

#[derive(Default, Debug, Serialize)]
pub struct History {
    pub channel: Option<Channel>,
    pub participants: BTreeMap<String, User>,
    pub events: VecDeque<Event>,
    pub current_call: Option<CallInfo>,
    pub archives: VecDeque<CallArchive>,
    #[serde(skip)]
    sequence: u64,
    #[serde(skip)]
    call_sequence: u64,
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
                if !same_channel {
                    if let Some(channel) = &channel {
                        if let Some(mut call) = self.current_call.take() {
                            call.ended_at.get_or_insert(timestamp);
                            self.archives.push_back(CallArchive {
                                call,
                                events: std::mem::take(&mut self.events),
                            });
                            if self.archives.len() > ARCHIVE_LIMIT {
                                self.archives.pop_front();
                            }
                        }
                        self.call_sequence += 1;
                        self.current_call = Some(CallInfo {
                            id: self.call_sequence,
                            channel: channel.clone(),
                            started_at: timestamp,
                            ended_at: None,
                        });
                    } else if let Some(call) = &mut self.current_call {
                        call.ended_at.get_or_insert(timestamp);
                    }
                }
                if same_channel && let Some(channel) = &channel {
                    if let Some(call) = &mut self.current_call {
                        call.channel = channel.clone();
                    }
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
                self.trim_events();
            }
        }
        Ok(())
    }

    fn trim_events(&mut self) {
        let total = self.events.len()
            + self
                .archives
                .iter()
                .map(|archive| archive.events.len())
                .sum::<usize>();
        let mut excess = total.saturating_sub(HISTORY_LIMIT);
        for archive in &mut self.archives {
            let remove = excess.min(archive.events.len());
            archive.events.drain(..remove);
            excess -= remove;
        }
        self.events.drain(..excess);
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
    fn joining_a_new_call_archives_previous_events_and_establishes_a_baseline() {
        let mut history = History::default();
        history.apply(snapshot(Some("100"), &["1"]), 1).unwrap();
        history
            .apply(snapshot(Some("100"), &["1", "9007199254740993"]), 2)
            .unwrap();
        history.apply(snapshot(Some("200"), &["3"]), 3).unwrap();
        assert!(history.events.is_empty());
        assert_eq!(history.archives.len(), 1);
        let archive = &history.archives[0];
        assert_eq!(archive.call.channel.id, "100");
        assert_eq!(archive.call.started_at, 1);
        assert_eq!(archive.call.ended_at, Some(3));
        assert_eq!(archive.events[0].user.id, "9007199254740993");
        assert_eq!(archive.events[0].action, Action::Joined);
        assert_eq!(history.current_call.as_ref().unwrap().id, 2);
        assert!(history.participants.contains_key("3"));
        history.apply(snapshot(Some("200"), &[]), 4).unwrap();
        assert_eq!(history.events[0].action, Action::Left);
        history.apply(Command::Clear, 5).unwrap();
        assert!(history.events.is_empty());
        assert_eq!(history.archives[0].events.len(), 1);
    }

    #[test]
    fn disconnect_retains_history_until_rejoining_even_the_same_channel() {
        let mut history = History::default();
        history.apply(snapshot(Some("100"), &[]), 1).unwrap();
        history.apply(snapshot(Some("100"), &["1"]), 2).unwrap();
        history.apply(snapshot(None, &[]), 3).unwrap();
        history.apply(snapshot(None, &[]), 4).unwrap();
        assert_eq!(history.events.len(), 1);
        assert!(history.archives.is_empty());
        assert_eq!(history.current_call.as_ref().unwrap().ended_at, Some(3));
        assert!(history.apply(snapshot(Some("invalid"), &[]), 5).is_err());
        assert!(history.archives.is_empty());
        history.apply(snapshot(Some("100"), &["2"]), 6).unwrap();
        assert!(history.events.is_empty());
        assert_eq!(history.archives[0].call.ended_at, Some(3));
        assert_eq!(history.archives[0].events.len(), 1);
        assert_eq!(history.current_call.as_ref().unwrap().started_at, 6);
    }

    #[test]
    fn archives_and_total_retained_events_are_bounded() {
        let mut history = History::default();
        for channel in 1..=25 {
            history
                .apply(snapshot(Some(&channel.to_string()), &[]), channel * 100)
                .unwrap();
            for offset in 1..=30 {
                let ids = if offset % 2 == 0 { vec![] } else { vec!["1"] };
                history
                    .apply(
                        snapshot(Some(&channel.to_string()), &ids),
                        channel * 100 + offset,
                    )
                    .unwrap();
            }
        }
        assert_eq!(history.archives.len(), ARCHIVE_LIMIT);
        assert_eq!(
            history.events.len()
                + history
                    .archives
                    .iter()
                    .map(|archive| archive.events.len())
                    .sum::<usize>(),
            HISTORY_LIMIT
        );
        assert_eq!(history.archives.back().unwrap().call.channel.id, "24");
        let value = serde_json::to_value(&history).unwrap();
        assert_eq!(value["archives"][19]["channel"]["id"], "24");
        assert_eq!(value["current_call"]["channel"]["id"], "25");
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
