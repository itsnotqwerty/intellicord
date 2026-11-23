use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

pub mod acoustic;
#[cfg(unix)]
pub mod acoustic_capture;
pub mod detection;
pub mod ingestion;
pub mod observe;
pub mod platform;

pub const HISTORY_LIMIT: usize = 500;
pub const ARCHIVE_LIMIT: usize = 20;
pub const PARTICIPANT_LIMIT: usize = 10_000;
pub const RECONNECT_WINDOW_MS: u64 = 30_000;

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
    #[serde(default)]
    pub kind: Option<u8>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reconnect_of: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reconnect_gap_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct CallRecap {
    pub baseline_count: usize,
    pub participant_ids: BTreeSet<String>,
    pub participant_ids_truncated: bool,
    pub peak_participants: usize,
    pub joined_count: u64,
    pub left_count: u64,
    pub probable_reconnections: u64,
}

impl CallRecap {
    fn new(participants: &BTreeMap<String, User>) -> Self {
        Self {
            baseline_count: participants.len(),
            participant_ids: participants.keys().cloned().collect(),
            participant_ids_truncated: false,
            peak_participants: participants.len(),
            joined_count: 0,
            left_count: 0,
            probable_reconnections: 0,
        }
    }

    fn observe(&mut self, participants: &BTreeMap<String, User>) {
        self.peak_participants = self.peak_participants.max(participants.len());
        for id in participants.keys() {
            if self.participant_ids.len() == PARTICIPANT_LIMIT {
                if !self.participant_ids.contains(id) {
                    self.participant_ids_truncated = true;
                    break;
                }
            } else {
                self.participant_ids.insert(id.clone());
            }
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CallInfo {
    pub id: u64,
    pub channel: Channel,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recap: Option<CallRecap>,
}

#[derive(Debug)]
struct PendingLeave {
    sequence: u64,
    timestamp: u64,
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
    #[serde(rename = "latest_sequence")]
    sequence: u64,
    #[serde(skip)]
    call_sequence: u64,
    #[serde(skip)]
    pending_leaves: BTreeMap<String, PendingLeave>,
}

impl History {
    pub fn apply(&mut self, command: Command, timestamp: u64) -> Result<(), &'static str> {
        match command {
            Command::Clear => self.events.clear(),
            Command::Snapshot { channel, users } => {
                if channel.as_ref().is_some_and(|value| !valid_id(&value.id))
                    || users.iter().any(|user| !valid_id(&user.id))
                    || users.len() > PARTICIPANT_LIMIT
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
                    self.pending_leaves.clear();
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
                            recap: (channel.kind == Some(3)).then(|| CallRecap::new(&participants)),
                        });
                    } else if let Some(call) = &mut self.current_call {
                        call.ended_at.get_or_insert(timestamp);
                    }
                }
                if same_channel && let Some(channel) = &channel {
                    if let Some(call) = &mut self.current_call {
                        call.channel = channel.clone();
                        if channel.kind != Some(3) {
                            call.recap = None;
                        }
                        if let Some(recap) = &mut call.recap {
                            recap.observe(&participants);
                        }
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
                        let mut reconnect_of = None;
                        let mut reconnect_gap_ms = None;
                        match action {
                            Action::Left => {
                                if self.pending_leaves.len() == PARTICIPANT_LIMIT
                                    && let Some(oldest) = self
                                        .pending_leaves
                                        .iter()
                                        .min_by_key(|(_, leave)| leave.sequence)
                                        .map(|(id, _)| id.clone())
                                {
                                    self.pending_leaves.remove(&oldest);
                                }
                                self.pending_leaves.insert(
                                    user.id.clone(),
                                    PendingLeave {
                                        sequence: self.sequence,
                                        timestamp,
                                    },
                                );
                            }
                            Action::Joined => {
                                if let Some(leave) = self.pending_leaves.remove(&user.id)
                                    && let Some(gap) = timestamp.checked_sub(leave.timestamp)
                                    && gap <= RECONNECT_WINDOW_MS
                                {
                                    reconnect_of = Some(leave.sequence);
                                    reconnect_gap_ms = Some(gap);
                                }
                            }
                        }
                        if let Some(recap) = self
                            .current_call
                            .as_mut()
                            .and_then(|call| call.recap.as_mut())
                        {
                            match action {
                                Action::Left => recap.left_count += 1,
                                Action::Joined => {
                                    recap.joined_count += 1;
                                    if reconnect_of.is_some() {
                                        recap.probable_reconnections += 1;
                                    }
                                }
                            }
                        }
                        self.events.push_back(Event {
                            sequence: self.sequence,
                            timestamp,
                            action,
                            user,
                            channel: channel.clone(),
                            reconnect_of,
                            reconnect_gap_ms,
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
                kind: None,
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

    fn group_snapshot(channel: Option<&str>, ids: &[&str]) -> Command {
        snapshot_with_kind(channel, ids, Some(3))
    }

    fn snapshot_with_kind(channel: Option<&str>, ids: &[&str], kind: Option<u8>) -> Command {
        let mut command = snapshot(channel, ids);
        if let Command::Snapshot {
            channel: Some(channel),
            ..
        } = &mut command
        {
            channel.kind = kind;
        }
        command
    }

    fn recap(history: &History) -> &CallRecap {
        history
            .current_call
            .as_ref()
            .unwrap()
            .recap
            .as_ref()
            .unwrap()
    }

    #[test]
    fn recap_requires_group_kind_at_baseline_and_stays_disabled_after_kind_changes() {
        let legacy: Channel = serde_json::from_str(r#"{"id":"100","name":"Lounge"}"#).unwrap();
        assert_eq!(legacy.kind, None);
        for kind in [None, Some(0), Some(1), Some(2), Some(3), Some(99)] {
            let mut history = History::default();
            let mut command = snapshot(Some("100"), &["1"]);
            if let Command::Snapshot {
                channel: Some(channel),
                ..
            } = &mut command
            {
                channel.kind = kind;
            }
            history.apply(command, 0).unwrap();
            assert_eq!(
                history.current_call.as_ref().unwrap().recap.is_some(),
                kind == Some(3)
            );
            history
                .apply(group_snapshot(Some("100"), &["1", "2"]), 1)
                .unwrap();
            assert_eq!(
                history.current_call.as_ref().unwrap().recap.is_some(),
                kind == Some(3)
            );
            history.apply(snapshot(Some("100"), &["1"]), 2).unwrap();
            assert!(history.current_call.as_ref().unwrap().recap.is_none());
            history
                .apply(group_snapshot(Some("100"), &["1", "2"]), 3)
                .unwrap();
            assert!(history.current_call.as_ref().unwrap().recap.is_none());
            assert_eq!(history.events[2].reconnect_of, Some(2));
            assert_eq!(history.events[2].reconnect_gap_ms, Some(1));
        }
    }

    #[test]
    fn reconnects_work_for_all_channel_kinds_across_unchanged_snapshots() {
        for kind in [None, Some(0), Some(1), Some(2), Some(3), Some(99)] {
            for (joined_at, expected) in [
                (100, Some(0)),
                (30_100, Some(30_000)),
                (30_101, None),
                (99, None),
            ] {
                let mut history = History::default();
                history
                    .apply(snapshot_with_kind(Some("100"), &["1"], kind), 0)
                    .unwrap();
                history
                    .apply(snapshot_with_kind(Some("100"), &[], kind), 100)
                    .unwrap();
                history
                    .apply(snapshot_with_kind(Some("100"), &[], kind), 100)
                    .unwrap();
                assert_eq!(history.pending_leaves.len(), 1);
                history
                    .apply(
                        snapshot_with_kind(Some("100"), &["1", "2"], kind),
                        joined_at,
                    )
                    .unwrap();
                assert_eq!(history.events.len(), 3);
                assert_eq!(history.events[0].action, Action::Left);
                assert_eq!(history.events[1].action, Action::Joined);
                assert_eq!(history.events[1].reconnect_of, expected.map(|_| 1));
                assert_eq!(history.events[1].reconnect_gap_ms, expected);
                assert_eq!(history.events[2].reconnect_of, None);
                assert!(history.pending_leaves.is_empty());
                assert_eq!(
                    history.current_call.as_ref().unwrap().recap.is_some(),
                    kind == Some(3)
                );
                if kind == Some(3) {
                    assert_eq!(
                        recap(&history).probable_reconnections,
                        u64::from(expected.is_some())
                    );
                }
            }
        }
    }

    #[test]
    fn nongroup_reconnect_tracking_survives_clear_but_not_call_transitions() {
        for kind in [None, Some(1), Some(2)] {
            for destination in [Some("200"), None] {
                let mut history = History::default();
                history
                    .apply(snapshot_with_kind(Some("100"), &["1"], kind), 0)
                    .unwrap();
                history
                    .apply(snapshot_with_kind(Some("100"), &[], kind), 1)
                    .unwrap();
                history.apply(Command::Clear, 2).unwrap();
                history
                    .apply(snapshot_with_kind(Some("100"), &["1"], kind), 3)
                    .unwrap();
                assert_eq!(history.events[0].reconnect_of, Some(1));
                history
                    .apply(snapshot_with_kind(Some("100"), &[], kind), 4)
                    .unwrap();
                history
                    .apply(snapshot_with_kind(destination, &[], kind), 5)
                    .unwrap();
                assert!(history.pending_leaves.is_empty());
                history
                    .apply(snapshot_with_kind(Some("100"), &[], kind), 6)
                    .unwrap();
                history
                    .apply(snapshot_with_kind(Some("100"), &["1"], kind), 7)
                    .unwrap();
                assert_eq!(history.events[0].reconnect_of, None);
                assert!(history.current_call.as_ref().unwrap().recap.is_none());
            }
        }
    }

    #[test]
    fn group_recap_counts_baseline_uniques_peak_and_raw_changes() {
        let mut history = History::default();
        history
            .apply(
                group_snapshot(Some("100"), &["9007199254740993", "2", "2"]),
                10,
            )
            .unwrap();
        assert!(history.events.is_empty());
        assert_eq!(recap(&history).baseline_count, 2);
        assert_eq!(recap(&history).joined_count, 0);
        history
            .apply(
                group_snapshot(Some("100"), &["9007199254740993", "2", "3"]),
                20,
            )
            .unwrap();
        history
            .apply(group_snapshot(Some("100"), &["3"]), 30)
            .unwrap();
        history
            .apply(group_snapshot(Some("100"), &["2", "3"]), 40)
            .unwrap();
        history
            .apply(group_snapshot(Some("100"), &["2", "3"]), 50)
            .unwrap();
        let stats = recap(&history);
        assert_eq!(stats.peak_participants, 3);
        assert_eq!(
            stats.participant_ids,
            BTreeSet::from(["9007199254740993".into(), "2".into(), "3".into()])
        );
        assert_eq!(
            (
                stats.joined_count,
                stats.left_count,
                stats.probable_reconnections
            ),
            (2, 2, 1)
        );
        assert_eq!(history.events.len(), 4);
        let value = serde_json::to_value(&history).unwrap();
        assert!(
            value["current_call"]["recap"]["participant_ids"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("9007199254740993"))
        );
        assert!(value["events"][0].get("reconnect_of").is_none());
        assert!(value["events"][0].get("reconnect_gap_ms").is_none());
        assert_eq!(value["events"][3]["reconnect_of"], 2);
        assert_eq!(value["events"][3]["reconnect_gap_ms"], 10);
    }

    #[test]
    fn reconnect_window_is_inclusive_nonnegative_and_same_user_only() {
        for (joined_at, expected) in [
            (100, Some(0)),
            (30_100, Some(30_000)),
            (30_101, None),
            (99, None),
        ] {
            let mut history = History::default();
            history
                .apply(group_snapshot(Some("100"), &["1"]), 0)
                .unwrap();
            history
                .apply(group_snapshot(Some("100"), &[]), 100)
                .unwrap();
            history
                .apply(group_snapshot(Some("100"), &["1", "2"]), joined_at)
                .unwrap();
            assert_eq!(history.events[1].reconnect_gap_ms, expected);
            assert_eq!(history.events[1].reconnect_of, expected.map(|_| 1));
            assert_eq!(history.events[2].reconnect_of, None);
            assert_eq!(
                recap(&history).probable_reconnections,
                u64::from(expected.is_some())
            );
            history
                .apply(group_snapshot(Some("100"), &[]), 40_000)
                .unwrap();
            history
                .apply(group_snapshot(Some("100"), &["1"]), 40_010)
                .unwrap();
            assert_eq!(history.events.back().unwrap().reconnect_of, Some(4));
        }
    }

    #[test]
    fn recap_and_reconnect_tracking_survive_event_trim_and_clear() {
        let mut history = History::default();
        history
            .apply(group_snapshot(Some("100"), &["1"]), 0)
            .unwrap();
        history.apply(group_snapshot(Some("100"), &[]), 1).unwrap();
        for timestamp in 2..=601 {
            let ids = if timestamp % 2 == 0 {
                vec!["2"]
            } else {
                vec![]
            };
            history
                .apply(group_snapshot(Some("100"), &ids), timestamp)
                .unwrap();
        }
        assert_eq!(history.events.len(), HISTORY_LIMIT);
        assert!(history.events.front().unwrap().sequence > 1);
        history
            .apply(group_snapshot(Some("100"), &["1"]), 602)
            .unwrap();
        assert_eq!(history.events.back().unwrap().reconnect_of, Some(1));
        assert_eq!(history.events.back().unwrap().reconnect_gap_ms, Some(601));
        assert_eq!(
            (
                recap(&history).joined_count,
                recap(&history).left_count,
                recap(&history).probable_reconnections
            ),
            (301, 301, 300)
        );
        history
            .apply(group_snapshot(Some("100"), &[]), 603)
            .unwrap();
        let leave_sequence = history.events.back().unwrap().sequence;
        history.apply(Command::Clear, 604).unwrap();
        assert!(history.events.is_empty());
        history
            .apply(group_snapshot(Some("100"), &["1"]), 605)
            .unwrap();
        assert_eq!(history.events.len(), 1);
        assert_eq!(history.events[0].reconnect_of, Some(leave_sequence));
        assert_eq!(history.events[0].reconnect_gap_ms, Some(2));
        let stats = recap(&history);
        assert_eq!(
            (
                stats.baseline_count,
                stats.peak_participants,
                stats.participant_ids.len()
            ),
            (1, 1, 2)
        );
        assert_eq!(
            (
                stats.joined_count,
                stats.left_count,
                stats.probable_reconnections
            ),
            (302, 302, 301)
        );
    }

    #[test]
    fn channel_transitions_reset_pending_leaves_including_disconnect_and_rejoin() {
        for destination in [Some("200"), None] {
            let mut history = History::default();
            history
                .apply(group_snapshot(Some("100"), &["1"]), 0)
                .unwrap();
            history.apply(group_snapshot(Some("100"), &[]), 1).unwrap();
            assert_eq!(history.pending_leaves.len(), 1);
            history.apply(group_snapshot(destination, &[]), 2).unwrap();
            assert!(history.pending_leaves.is_empty());
            history.apply(group_snapshot(Some("100"), &[]), 3).unwrap();
            history
                .apply(group_snapshot(Some("100"), &["1"]), 4)
                .unwrap();
            assert_eq!(history.events.len(), 1);
            assert_eq!(history.events[0].reconnect_of, None);
            assert_eq!(recap(&history).probable_reconnections, 0);
            assert_eq!(recap(&history).joined_count, 1);
            assert_eq!(history.archives[0].call.ended_at, Some(2));
            assert_eq!(
                history.archives[0].call.recap.as_ref().unwrap().left_count,
                1
            );
        }
    }

    #[test]
    fn archived_recap_is_independent_of_retained_events_and_new_calls() {
        let mut history = History::default();
        history
            .apply(group_snapshot(Some("100"), &["1"]), 10)
            .unwrap();
        history.apply(group_snapshot(Some("100"), &[]), 20).unwrap();
        history
            .apply(group_snapshot(Some("100"), &["1", "2"]), 30)
            .unwrap();
        history.apply(group_snapshot(Some("200"), &[]), 40).unwrap();
        for timestamp in 41..=641 {
            let ids = if timestamp % 2 == 0 {
                vec![]
            } else {
                vec!["3"]
            };
            history
                .apply(group_snapshot(Some("200"), &ids), timestamp)
                .unwrap();
        }
        assert!(history.archives[0].events.is_empty());
        history.apply(Command::Clear, 642).unwrap();
        let archive = &history.archives[0];
        assert_eq!(
            (archive.call.started_at, archive.call.ended_at),
            (10, Some(40))
        );
        let stats = archive.call.recap.as_ref().unwrap();
        assert_eq!((stats.baseline_count, stats.peak_participants), (1, 2));
        assert_eq!(
            stats.participant_ids,
            BTreeSet::from(["1".into(), "2".into()])
        );
        assert_eq!(
            (
                stats.joined_count,
                stats.left_count,
                stats.probable_reconnections
            ),
            (2, 1, 1)
        );
        let value = serde_json::to_value(&history).unwrap();
        assert_eq!(value["archives"][0]["recap"]["probable_reconnections"], 1);
        assert_eq!(value["archives"][0]["started_at"], 10);
        assert_eq!(value["archives"][0]["ended_at"], 40);
        assert!(value.get("pending_leaves").is_none());
    }

    #[test]
    fn unique_ids_and_pending_leaves_are_bounded_without_capping_counters() {
        let mut history = History::default();
        let ids: Vec<_> = (1..=PARTICIPANT_LIMIT).map(|id| id.to_string()).collect();
        let references: Vec<_> = ids.iter().map(String::as_str).collect();
        history
            .apply(group_snapshot(Some("100"), &references), 0)
            .unwrap();
        assert!(!recap(&history).participant_ids_truncated);
        history
            .apply(group_snapshot(Some("100"), &references), 0)
            .unwrap();
        assert!(!recap(&history).participant_ids_truncated);
        history.apply(group_snapshot(Some("100"), &[]), 1).unwrap();
        assert_eq!(history.pending_leaves.len(), PARTICIPANT_LIMIT);
        history
            .apply(group_snapshot(Some("100"), &["10001"]), 2)
            .unwrap();
        history.apply(group_snapshot(Some("100"), &[]), 3).unwrap();
        assert_eq!(history.pending_leaves.len(), PARTICIPANT_LIMIT);
        assert!(!history.pending_leaves.contains_key("1"));
        assert!(history.pending_leaves.contains_key("2"));
        history
            .apply(group_snapshot(Some("100"), &["1", "2", "10001"]), 4)
            .unwrap();
        let stats = recap(&history);
        assert_eq!(
            (
                stats.baseline_count,
                stats.peak_participants,
                stats.participant_ids.len()
            ),
            (PARTICIPANT_LIMIT, PARTICIPANT_LIMIT, PARTICIPANT_LIMIT)
        );
        assert!(stats.participant_ids.contains("1"));
        assert!(!stats.participant_ids.contains("10001"));
        assert!(stats.participant_ids_truncated);
        assert_eq!(
            (
                stats.joined_count,
                stats.left_count,
                stats.probable_reconnections
            ),
            (4, 10_001, 2)
        );
        assert_eq!(history.events[history.events.len() - 3].reconnect_of, None);
        let value = serde_json::to_value(&history).unwrap();
        assert_eq!(
            value["current_call"]["recap"]["participant_ids_truncated"],
            true
        );
        assert_eq!(
            value["current_call"]["recap"]["participant_ids"]
                .as_array()
                .unwrap()
                .len(),
            PARTICIPANT_LIMIT
        );
        assert_eq!(value["events"].as_array().unwrap().len(), HISTORY_LIMIT);
    }

    #[test]
    fn unique_id_truncation_detects_extras_after_known_ids_and_stays_set() {
        let participants: BTreeMap<_, _> = (1..=PARTICIPANT_LIMIT)
            .map(|id| {
                let id = id.to_string();
                let user = User {
                    id: id.clone(),
                    username: id.clone(),
                    display_name: id.clone(),
                    avatar_url: None,
                };
                (id, user)
            })
            .collect();
        let mut stats = CallRecap::new(&participants);
        let mut observed = participants.clone();
        observed.remove("9999");
        let mut extra = participants["1"].clone();
        extra.id = "99999".into();
        observed.insert(extra.id.clone(), extra);
        stats.observe(&observed);
        assert!(stats.participant_ids_truncated);
        assert_eq!(stats.participant_ids.len(), PARTICIPANT_LIMIT);
        stats.observe(&participants);
        assert!(stats.participant_ids_truncated);
    }

    #[test]
    fn latest_sequence_is_serialized_across_trimming_clear_and_call_transitions() {
        let mut history = History::default();
        assert_eq!(
            serde_json::to_value(&history).unwrap()["latest_sequence"],
            0
        );
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
        let value = serde_json::to_value(&history).unwrap();
        assert_eq!(value["latest_sequence"], 600);
        assert!(value.get("sequence").is_none());
        assert_eq!(history.events.len(), HISTORY_LIMIT);
        history.apply(Command::Clear, 601).unwrap();
        assert_eq!(
            serde_json::to_value(&history).unwrap()["latest_sequence"],
            600
        );
        history.apply(snapshot(Some("200"), &[]), 602).unwrap();
        assert_eq!(
            serde_json::to_value(&history).unwrap()["latest_sequence"],
            600
        );
        history.apply(snapshot(Some("200"), &["1"]), 603).unwrap();
        assert_eq!(
            serde_json::to_value(&history).unwrap()["latest_sequence"],
            601
        );
    }

    #[test]
    fn invalid_snapshot_does_not_mutate_recap_or_pending_reconnections() {
        let mut history = History::default();
        history
            .apply(group_snapshot(Some("100"), &["1"]), 0)
            .unwrap();
        history.apply(group_snapshot(Some("100"), &[]), 1).unwrap();
        let before = serde_json::to_value(&history).unwrap();
        assert!(
            history
                .apply(group_snapshot(Some("200"), &["invalid"]), 2)
                .is_err()
        );
        assert_eq!(serde_json::to_value(&history).unwrap(), before);
        assert_eq!(history.pending_leaves.len(), 1);
        history
            .apply(group_snapshot(Some("100"), &["1"]), 3)
            .unwrap();
        assert_eq!(history.events.back().unwrap().reconnect_of, Some(1));
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
