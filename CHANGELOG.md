# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.14.6](https://github.com/openssh-rust/openssh-sftp-client/compare/openssh-sftp-client-v0.14.5...openssh-sftp-client-v0.14.6) - 2024-07-25

### Other
- Fix panic when flush_interval is set to 0 ([#136](https://github.com/openssh-rust/openssh-sftp-client/pull/136))

## [0.14.5](https://github.com/openssh-rust/openssh-sftp-client/compare/openssh-sftp-client-v0.14.4...openssh-sftp-client-v0.14.5) - 2024-07-11

### Other
- Implement `Sftp::from_clonable_session*` ([#131](https://github.com/openssh-rust/openssh-sftp-client/pull/131))

## [0.14.4](https://github.com/openssh-rust/openssh-sftp-client/compare/openssh-sftp-client-v0.14.3...openssh-sftp-client-v0.14.4) - 2024-06-27

### Other
- Run rust.yml on merge_queue ([#128](https://github.com/openssh-rust/openssh-sftp-client/pull/128))
- Impl `Default` for `Permissions` ([#126](https://github.com/openssh-rust/openssh-sftp-client/pull/126))
- Use release-plz in publish.yml ([#125](https://github.com/openssh-rust/openssh-sftp-client/pull/125))
- Support setting time in MetaDataBuilder ([#124](https://github.com/openssh-rust/openssh-sftp-client/pull/124))
The changelog for this crate is kept in the project's Rust documentation in the changelog module.
