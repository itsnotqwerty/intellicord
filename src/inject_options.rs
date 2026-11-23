use std::path::PathBuf;

use anyhow::{Context, Result, bail, ensure};

#[derive(Clone)]
pub struct Observation {
    pub object: PathBuf,
    pub process_id: u32,
    pub seconds: u64,
}

#[derive(Clone)]
pub struct Options {
    pub port: u16,
    pub target: Option<String>,
    pub observation: Option<Observation>,
    pub acoustic: Option<Acoustic>,
}

#[derive(Clone)]
pub struct Acoustic {
    pub serial: u64,
    pub seconds: u64,
    pub defer_start: bool,
}

impl Options {
    pub fn parse(arguments: &[String]) -> Result<Self> {
        let mut port = None;
        let mut target = None;
        let mut object = None;
        let mut process_id = None;
        let mut seconds = None;
        let mut acknowledged = false;
        let mut acoustic_serial = None;
        let mut acoustic_seconds = None;
        let mut acoustic_acknowledged = false;
        let mut acoustic_deferred = false;
        let mut arguments = arguments.iter();
        while let Some(option) = arguments.next() {
            if option == "--acknowledge-injected-output-capture" {
                ensure!(!acoustic_acknowledged, "Duplicate acoustic acknowledgement");
                acoustic_acknowledged = true;
                continue;
            }
            if option == "--defer-acoustic-start" {
                ensure!(!acoustic_deferred, "Duplicate deferred acoustic start");
                acoustic_deferred = true;
                continue;
            }
            if option == "--acknowledge-injected-observation" {
                ensure!(!acknowledged, "Duplicate observation acknowledgement");
                acknowledged = true;
                continue;
            }
            ensure!(
                matches!(
                    option.as_str(),
                    "--port"
                        | "--target"
                        | "--observe-object"
                        | "--observe-pid"
                        | "--observe-seconds"
                        | "--acoustic-sink-serial"
                        | "--acoustic-seconds"
                ),
                "Unknown inject option: {option}"
            );
            let value = arguments.next().context("Inject options require values")?;
            ensure!(
                !value.is_empty() && !value.starts_with("--"),
                "Inject options require nonempty values"
            );
            match option.as_str() {
                "--acoustic-sink-serial" => {
                    ensure!(acoustic_serial.is_none(), "Duplicate acoustic sink");
                    acoustic_serial = Some(
                        value
                            .parse::<u64>()
                            .context("Invalid acoustic sink serial")?,
                    );
                }
                "--acoustic-seconds" => {
                    ensure!(acoustic_seconds.is_none(), "Duplicate acoustic duration");
                    acoustic_seconds =
                        Some(value.parse::<u64>().context("Invalid acoustic duration")?);
                }
                "--port" => {
                    ensure!(port.is_none(), "Duplicate port");
                    port = Some(value.parse::<u16>().context("Invalid port")?);
                }
                "--target" => {
                    ensure!(target.is_none(), "Duplicate target");
                    target = Some(value.clone());
                }
                "--observe-object" => {
                    ensure!(object.is_none(), "Duplicate observation object");
                    object = Some(PathBuf::from(value));
                }
                "--observe-pid" => {
                    ensure!(process_id.is_none(), "Duplicate observation PID");
                    process_id = Some(value.parse::<u32>().context("Invalid observation PID")?);
                }
                "--observe-seconds" => {
                    ensure!(seconds.is_none(), "Duplicate observation duration");
                    seconds = Some(
                        value
                            .parse::<u64>()
                            .context("Invalid observation duration")?,
                    );
                }
                _ => unreachable!(),
            }
        }
        let port = port.unwrap_or(9222);
        ensure!(port != 0, "Port must be nonzero");
        let requested =
            object.is_some() || process_id.is_some() || seconds.is_some() || acknowledged;
        let observation = if requested {
            ensure!(
                acknowledged,
                "Injected observation requires --acknowledge-injected-observation"
            );
            let (Some(object), Some(process_id), Some(seconds)) = (object, process_id, seconds)
            else {
                bail!(
                    "Injected observation requires --observe-object, --observe-pid and --observe-seconds"
                );
            };
            ensure!(
                (1..=i32::MAX as u32).contains(&process_id) && (1..=60).contains(&seconds),
                "Invalid observation target or duration"
            );
            ensure!(
                cfg!(all(target_os = "linux", feature = "observe")),
                "Injected observation requires Linux and the observe feature"
            );
            Some(Observation {
                object,
                process_id,
                seconds,
            })
        } else {
            None
        };
        let acoustic = if acoustic_serial.is_some()
            || acoustic_seconds.is_some()
            || acoustic_acknowledged
            || acoustic_deferred
        {
            ensure!(
                acoustic_acknowledged,
                "Injected output capture requires --acknowledge-injected-output-capture"
            );
            let (Some(serial), Some(seconds)) = (acoustic_serial, acoustic_seconds) else {
                bail!(
                    "Injected output capture requires --acoustic-sink-serial and --acoustic-seconds"
                );
            };
            ensure!(
                cfg!(target_os = "linux") && cfg!(target_endian = "little"),
                "Injected output capture requires little-endian Linux"
            );
            ensure!(
                serial > 0 && (1..=60).contains(&seconds),
                "Invalid acoustic sink or duration"
            );
            Some(Acoustic {
                serial,
                seconds,
                defer_start: acoustic_deferred,
            })
        } else {
            None
        };
        Ok(Self {
            port,
            target,
            observation,
            acoustic,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Result<Options> {
        Options::parse(
            &arguments
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn acoustic_capture_requires_its_own_complete_consent_policy() {
        let complete = [
            "--acoustic-sink-serial",
            "42",
            "--acoustic-seconds",
            "1",
            "--acknowledge-injected-output-capture",
        ];
        assert!(parse(&[]).unwrap().acoustic.is_none());
        for count in 1..complete.len() {
            assert!(parse(&complete[..count]).is_err());
        }
        for invalid in [
            vec!["--acknowledge-injected-output-capture"],
            vec!["--acoustic-sink-serial", "--acoustic-seconds"],
            vec![
                "--acoustic-sink-serial",
                "42",
                "--acoustic-seconds",
                "1",
                "--acknowledge-injected-observation",
            ],
        ] {
            assert!(parse(&invalid).is_err());
        }
        for (index, value) in [(1, "0"), (3, "0"), (3, "61")] {
            let mut invalid = complete;
            invalid[index] = value;
            assert!(parse(&invalid).is_err());
        }
        let mut duplicate = complete.to_vec();
        duplicate.push("--acknowledge-injected-output-capture");
        assert!(parse(&duplicate).is_err());
        if cfg!(target_os = "linux") && cfg!(target_endian = "little") {
            let acoustic = parse(&complete).unwrap().acoustic.unwrap();
            assert_eq!(acoustic.serial, 42);
            assert_eq!(acoustic.seconds, 1);
            assert!(!acoustic.defer_start);
            let deferred = parse(&[
                "--acoustic-sink-serial",
                "42",
                "--acoustic-seconds",
                "1",
                "--acknowledge-injected-output-capture",
                "--defer-acoustic-start",
            ])
            .unwrap()
            .acoustic
            .unwrap();
            assert!(deferred.defer_start);
            assert!(parse(&["--defer-acoustic-start"]).is_err());
            assert!(
                parse(&[
                    "--acoustic-sink-serial",
                    "42",
                    "--acoustic-seconds",
                    "1",
                    "--acknowledge-injected-output-capture",
                    "--defer-acoustic-start",
                    "--defer-acoustic-start",
                ])
                .is_err()
            );
        } else {
            assert!(parse(&complete).is_err());
        }
    }

    #[test]
    fn ordinary_injection_has_no_observation() {
        let options = parse(&[]).unwrap();
        assert_eq!(options.port, 9222);
        assert!(options.observation.is_none());
        let options = parse(&["--target", "page", "--port", "9223"]).unwrap();
        assert_eq!(options.port, 9223);
        assert_eq!(options.target.as_deref(), Some("page"));
        assert!(options.observation.is_none());
    }

    #[test]
    fn injected_observation_is_all_or_nothing_and_not_implicitly_enabled() {
        let complete = [
            "--observe-object",
            "/dev/null",
            "--observe-pid",
            "1",
            "--observe-seconds",
            "1",
            "--acknowledge-injected-observation",
        ];
        for count in 1..complete.len() {
            assert!(parse(&complete[..count]).is_err());
        }
        assert!(parse(&["--acknowledge-injected-observation"]).is_err());
        assert!(parse(&["--port", "0"]).is_err());
        assert!(parse(&["--target", ""]).is_err());
        assert!(parse(&["--port", "9222", "--port", "9223"]).is_err());
        assert!(parse(&["--observe-object", "--observe-pid"]).is_err());
        for (index, replacement) in [(3, "0"), (3, "2147483648"), (5, "0"), (5, "61")] {
            let mut invalid = complete;
            invalid[index] = replacement;
            assert!(parse(&invalid).is_err());
        }
        let result = parse(&complete);
        if cfg!(all(target_os = "linux", feature = "observe")) {
            let observation = result.unwrap().observation.unwrap();
            assert_eq!(observation.object, PathBuf::from("/dev/null"));
            assert_eq!(observation.process_id, 1);
            assert_eq!(observation.seconds, 1);
        } else {
            assert!(result.is_err());
        }
        let mut duplicate = complete.to_vec();
        duplicate.push("--acknowledge-injected-observation");
        assert!(parse(&duplicate).is_err());
    }
}
