# MailKeep - Archive Your IMAP Mail

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1] - 2026-06-28

### Features

- _(cli)_ Wire ImapSubsystem, poll-interval config, 10s shutdown - ([4584627](https://github.com/szinn/MailKeep/commit/458462711b1f52347286ece4a2b21407247e4ea3))
- _(cli)_ Add --verbose to imap command (dump raw LIST entries) - ([d362732](https://github.com/szinn/MailKeep/commit/d3627322e470745802753c418e42e12cf0050fac))
- _(cli)_ Add 'imap' command to inspect a server's folders - ([7e43e3a](https://github.com/szinn/MailKeep/commit/7e43e3a29a1f8a4633ad661b2a350aca50de57e6))
- _(core)_ Publish AccountsChanged on account mutations - ([f8752ef](https://github.com/szinn/MailKeep/commit/f8752ef097552a8207d459a6f72ecc5711234df2))
- _(core)_ Add EventService broadcast bus - ([7a69443](https://github.com/szinn/MailKeep/commit/7a694436d1200cfe6a022737a7e2b02b17d77c78))
- _(core)_ Add ImapPort::tracked_accounts accessor - ([c3d72c6](https://github.com/szinn/MailKeep/commit/c3d72c60dc67408d53f1d1f099e16a24f0050bb5))
- _(core)_ Surface IMAP hierarchy delimiter on RemoteFolder - ([fed02c8](https://github.com/szinn/MailKeep/commit/fed02c88b781c9a7a9cb27e04eff5d21413014ca))
- _(core)_ Implement ImapAccountService lifecycle + status reconciliation - ([a642ff0](https://github.com/szinn/MailKeep/commit/a642ff01d1ecd12ecd733720bef8b91912d8f88a))
- _(core)_ Wire mk-imap adapter into mailkeep startup - ([91c3fb7](https://github.com/szinn/MailKeep/commit/91c3fb702208da4e197546aa0d9b8a3cb7b00354))
- _(core)_ Wire imap_port_factory into ExternalServices and CoreServices - ([e121319](https://github.com/szinn/MailKeep/commit/e121319cf4b154b78046a04a1cfb7d46b46cbb93))
- _(core)_ Add ImapAccountServiceImpl forwarding impl - ([cb17c96](https://github.com/szinn/MailKeep/commit/cb17c96008c52c44908ad7a284bd350c6cf5f102))
- _(core)_ Add imap port types, traits, and special-use mapping - ([1dbf678](https://github.com/szinn/MailKeep/commit/1dbf678139c2f93ecfbc7a121be54695bd9e6f5c))
- _(core)_ Fail jobs terminally on non-transient handler errors - ([c097683](https://github.com/szinn/MailKeep/commit/c0976839efb09d34b4a5f089aff943cadd1ed50e))
- _(core)_ Add ingest service and ParseMessageJob - ([7e24a7b](https://github.com/szinn/MailKeep/commit/7e24a7bb57bcec5e93f03227b3aea87036c5df02))
- _(core)_ JobService enqueue returns JobId - ([08ff833](https://github.com/szinn/MailKeep/commit/08ff8335f507dc6fb71c9b30d93f5deaf5693381))
- _(core)_ Add ContentHash serde and PRIORITY_INGEST constant - ([984e9d1](https://github.com/szinn/MailKeep/commit/984e9d1b5363c165b7701748e0efbd45d9349652))
- _(core)_ Implement MessageServiceImpl::record_parsed_message - ([276417b](https://github.com/szinn/MailKeep/commit/276417be656ffd0584e47f29ad2feab724826078))
- _(core)_ Implement FolderServiceImpl methods - ([765739c](https://github.com/szinn/MailKeep/commit/765739c2adb61b3020808ad29c22e206e8742dd5))
- _(core)_ Add folder domain module, adapter, and schema - ([af1590c](https://github.com/szinn/MailKeep/commit/af1590c62a76bef6d525090ca39b2a6cbf5beb83))
- _(core)_ Wire AccountService into CoreServices - ([1b1c000](https://github.com/szinn/MailKeep/commit/1b1c000d3bec01fd0b22a50167b630d794a29587))
- _(core)_ Add AccountService with envelope encryption - ([bf80c9c](https://github.com/szinn/MailKeep/commit/bf80c9c67ab338bc6f4f68d0cadf227b6308e330))
- _(core)_ Add AccountRepository port trait - ([bdc40f5](https://github.com/szinn/MailKeep/commit/bdc40f50088c6ff311f4907143a2c484c4e37c04))
- _(core)_ Finalise core::imap value types - ([e26682a](https://github.com/szinn/MailKeep/commit/e26682ad85b2412a7d9d8a681de1cb78320bbfb1))
- _(core)_ Add Account model with encrypted-credentials envelope - ([54ebae1](https://github.com/szinn/MailKeep/commit/54ebae10bc9b669e2afbfeed8b71cebe146247bc))
- _(core)_ JobWorker subscribes to wake notify (MK-16 part 2) - ([bd4954b](https://github.com/szinn/MailKeep/commit/bd4954b7521398c4448aafa54aa20a0546aafc18))
- _(core)_ JobService fires wake notify after enqueue commit (MK-16 part 1) - ([b8e985f](https://github.com/szinn/MailKeep/commit/b8e985fbcb90f5c414d2566f6af7e2cbc561d6e2))
- _(core)_ JobWorker subsystem with concurrency from config - ([f6bd539](https://github.com/szinn/MailKeep/commit/f6bd53990fb7c6cda5ad4a04d754c96d7433e6b1))
- _(core)_ Wire job_service into CoreServices via ExternalServices - ([a67df3c](https://github.com/szinn/MailKeep/commit/a67df3c6716da323c5128fa23732ea19029c8a79))
- _(core)_ Add JobService trait, impl, and handler registry - ([118f172](https://github.com/szinn/MailKeep/commit/118f1721d596d9bddfc1b7d53be277a38b5d9bce))
- _(core)_ Add JobRepository port + RepositoryService accessor - ([5bfa82c](https://github.com/szinn/MailKeep/commit/5bfa82c00717216f14f9bc6639a0417036135aec))
- _(core)_ Add jobs module skeleton with Job, JobStatus, JobHandler - ([2417529](https://github.com/szinn/MailKeep/commit/241752956dfd6e8b3072b5bd5b7e0639446d0198))
- _(core)_ Wire cipher + storage through CoreServices and main.rs - ([fb2871d](https://github.com/szinn/MailKeep/commit/fb2871dbf96aee19da113c834261533fcc5c972f))
- _(core)_ Add storage trait defs + Error::BlobNotFound - ([b95ffb8](https://github.com/szinn/MailKeep/commit/b95ffb86981135818ae8e335100f8934055fe671))
- _(core)_ Add crypto module with HKDF master key + ChaCha20-Poly1305 AEAD - ([fc3e06d](https://github.com/szinn/MailKeep/commit/fc3e06d455d8c469b7d70881b470755aa25a6534))
- _(core)_ Add account module stub with AccountId/AccountToken aliases - ([8fb8046](https://github.com/szinn/MailKeep/commit/8fb8046a9c64b1e17c99b8d21a0eda51f3fdf87b))
- _(core)_ Add ContentHash type for content-addressed storage - ([7cecad9](https://github.com/szinn/MailKeep/commit/7cecad904308ad87bec1325b681eac96057e87e2))
- _(core,database)_ Add message domain — models, traits, schema, adapters - ([fb7f56a](https://github.com/szinn/MailKeep/commit/fb7f56a4287131c9bae89739a9195d9a2b7cefd0))
- _(database)_ Create accounts table migration and entity - ([faddcec](https://github.com/szinn/MailKeep/commit/faddcec32678596b076da7d8a83ada4d6f6a3396))
- _(database)_ SeaORM jobs entity + migration + JobRepositoryAdapter - ([57f8b23](https://github.com/szinn/MailKeep/commit/57f8b23ddac1ea79d8284dcbdc76081603a2dbe2))
- _(database,core)_ Add AccountRepositoryAdapter and wire RepositoryService - ([0589bae](https://github.com/szinn/MailKeep/commit/0589baebf6042839f0578a6ecd473854d9124d00))
- _(frontend)_ Live account-status via SSE EventSource - ([52eaf2d](https://github.com/szinn/MailKeep/commit/52eaf2ddfd88a733105feca004d407c7715c5745))
- _(frontend)_ Add auth-gated SSE events endpoint - ([3efddf1](https://github.com/szinn/MailKeep/commit/3efddf12d8210bd8899afb3e12fdc87777662146))
- _(frontend)_ Compact account row with glyph status indicator - ([fee0f62](https://github.com/szinn/MailKeep/commit/fee0f622a71cecaca8451dd8647214275ad06903))
- _(frontend)_ MK-9 edit-folders modal + folder server fns - ([030a891](https://github.com/szinn/MailKeep/commit/030a89170f06e3b49dc548e9afb29f1f84fb7baf))
- _(frontend)_ MK-9 delete account with confirm modal - ([d733ab2](https://github.com/szinn/MailKeep/commit/d733ab2a61ddc1a6513203ba89fedde4dc247289))
- _(frontend)_ MK-9 enable/disable account via kebab menu - ([18ec405](https://github.com/szinn/MailKeep/commit/18ec405b816850e5be7ab1b6e1c4d5b895c2247b))
- _(frontend)_ MK-9 account list status badges + refresh + empty state - ([e50ae2d](https://github.com/szinn/MailKeep/commit/e50ae2dbd9318be22e84fadbb8672483d7777395))
- _(frontend)_ MK-9 account/folder DTOs + display helpers - ([8abead9](https://github.com/szinn/MailKeep/commit/8abead9a6a3fd64fbefe4e2f33108ee7a9b2dd38))
- _(frontend)_ Add home icon to nav bar - ([5d69268](https://github.com/szinn/MailKeep/commit/5d692687cbe21ad8d504e75e13cf5d5e9e2fdb73))
- _(frontend)_ Show total folder count in add-account picker header - ([fec5e7b](https://github.com/szinn/MailKeep/commit/fec5e7b70f62be97043eaa82248f958396413119))
- _(frontend)_ Home shell with account list and add button - ([e4fe725](https://github.com/szinn/MailKeep/commit/e4fe7253fd0bd62dc57a16954a24b9fe978551e6))
- _(frontend)_ Assemble account-add wizard page and route - ([23f43cb](https://github.com/szinn/MailKeep/commit/23f43cbac0f3222496f6573852f906a0701795df))
- _(frontend)_ Add folder picker tree component - ([571e1f9](https://github.com/szinn/MailKeep/commit/571e1f9380f6b1bbb3ed21b64e29a4128d863756))
- _(frontend)_ Add account connection form component - ([43620db](https://github.com/szinn/MailKeep/commit/43620db2d8e2376425720448bdb148a46bf413c7))
- _(frontend)_ Add tri-state folder-tree selection model - ([7c0a995](https://github.com/szinn/MailKeep/commit/7c0a99503c572d14827f1de8787748de38e2cf0b))
- _(frontend)_ Add create-and-start and list-accounts server functions - ([2936543](https://github.com/szinn/MailKeep/commit/2936543cfb0510969931b7c9d3b4a0febc028ee9))
- _(frontend)_ Add IMAP probe server functions for account add - ([8f63541](https://github.com/szinn/MailKeep/commit/8f63541e00449c4e86df080c3c0cc1bd29cbf54d))
- _(frontend)_ Add account-add DTOs and core mapping helpers - ([fbea4d0](https://github.com/szinn/MailKeep/commit/fbea4d0f330c603b44264b3d28bd81e20b9bf636))
- _(imap)_ UIDVALIDITY rollover resets cursor and drops stale locations - ([87c26eb](https://github.com/szinn/MailKeep/commit/87c26eb54d02ebf0b6b2cdd887ed193c10f27dee))
- _(imap)_ IDLE connection with EXISTS handling and backoff reconnect - ([990c2ac](https://github.com/szinn/MailKeep/commit/990c2acd65d15ac906912c55dd02cc25cfd14edb))
- _(imap)_ Poll task syncing non-idle folders on an interval - ([24d42c1](https://github.com/szinn/MailKeep/commit/24d42c1f49191deafd5adf67b802a27a31b1cf2e))
- _(imap)_ Per-account sync task scaffold with batched fetch + checkpoint - ([bb70d4d](https://github.com/szinn/MailKeep/commit/bb70d4dc4d66dd07b5146c98e9d59d6a7912533a))
- _(imap)_ Add IMAP adapter with test_connection and list_folders - ([ca72816](https://github.com/szinn/MailKeep/commit/ca728166d76777f6d5711be7cfb28142b2e3c3ec))
- _(parser)_ Wire handler registration and add end-to-end ingest tests - ([137955f](https://github.com/szinn/MailKeep/commit/137955f5568e2f03feb5ea5a794544a8d562c768))
- _(parser)_ Add ParseMessageHandler and register_handlers - ([ab242c6](https://github.com/szinn/MailKeep/commit/ab242c6f0c4e4e3476046b81a4093bf48186bc81))
- _(parser)_ Add mail-parser pure parsing layer - ([9f8bf5e](https://github.com/szinn/MailKeep/commit/9f8bf5ee2cd86923e0114126652fc292cb402faa))
- _(storage)_ Add mk-storage filesystem adapter crate - ([51c7653](https://github.com/szinn/MailKeep/commit/51c7653cada9a468f49bd905de11ce28059ec6ff))

### Bug Fixes

- _(core)_ Stop_all/reconcile cover tracked-set union - ([4bc87c0](https://github.com/szinn/MailKeep/commit/4bc87c008ee83fe600d78b0b135516ebefecb736))
- _(core)_ Treat SQLite busy/locked as transient so contention can't crash the worker - ([7968af7](https://github.com/szinn/MailKeep/commit/7968af7515f8e68cf6c8ff64b01069672336d16f))
- _(core)_ Deserialize credentials enum in start_one before IMAP login - ([bc88f77](https://github.com/szinn/MailKeep/commit/bc88f77a99013076254eec78a96c8677d8333925))
- _(database)_ Key message identity on content hash, not Message-ID - ([ec81a9d](https://github.com/szinn/MailKeep/commit/ec81a9de9fe540b8024740431affede68b25c9a0))
- _(database)_ Serialize SQLite access through a single pooled connection - ([e30a3ff](https://github.com/szinn/MailKeep/commit/e30a3ff8d6cbd9dc8deacc6420624c3872644842))
- _(frontend)_ Guard change_initial_password on pending forced change - ([d92185d](https://github.com/szinn/MailKeep/commit/d92185db7b30b32f98dfc2d56c1b5c06b7b0c7c9))
- _(frontend)_ Emit anti-flash script once at root, not per-navigation - ([c7e5280](https://github.com/szinn/MailKeep/commit/c7e528042d67d4a3ec49e0272f7422c979d60d1b))
- _(frontend)_ Stop spurious 'connection details changed' after probe - ([1564437](https://github.com/szinn/MailKeep/commit/15644379f725292307e6badef9e1a02c343ebb6d))

### Refactor

- _(core)_ Drain stop_all concurrently to stay within shutdown budget - ([e143701](https://github.com/szinn/MailKeep/commit/e143701ae7b857f5316d28ca1f16adb3977ddc90))
- _(core)_ JobWorker loop respects shutdown and skips inter-job pause (MK-13, MK-17) - ([8da1fa3](https://github.com/szinn/MailKeep/commit/8da1fa35bf70dbdb6606e2ebbdb24501b8bf1248))
- _(core)_ Expose CoreSubsystem; jobs becomes internal child - ([d1eb2f2](https://github.com/szinn/MailKeep/commit/d1eb2f2f39515fa25f736674f454ef6972139be4))
- _(core)_ Hide MasterKey from public crypto API - ([b786bd2](https://github.com/szinn/MailKeep/commit/b786bd2d176a1fc8b3600552567645d90d7444fb))

### Stying

- _(core)_ Resolve clippy lints in imap service (unused_result_ok, manual_let_else, redundant imports) - ([bff4803](https://github.com/szinn/MailKeep/commit/bff48033604b1bf6cf999dab4406e033c599a3c1))

### Testing

- _(core)_ Tighten jobs service test coverage (MK-10, MK-11) - ([c4ca463](https://github.com/szinn/MailKeep/commit/c4ca46327cf27497f812c4dca70a49f508b82635))
- _(database)_ Tighten JobRepository adapter test assertions (MK-12) - ([660499b](https://github.com/szinn/MailKeep/commit/660499bc285dbd77d4a4fb98f50f0d0d8cb3165f))
- _(frontend)_ MK-9 account-lifecycle greenmail integration tests - ([409bef2](https://github.com/szinn/MailKeep/commit/409bef2695fd403422482f919aa98309947787ba))
- _(frontend)_ Greenmail e2e for account add + first sync (shared greenmail harness) - ([c959986](https://github.com/szinn/MailKeep/commit/c959986770c051a262082a762c47cc40ab6ea982))
- _(imap)_ Greenmail integration tests for sync, IDLE, rollover, shutdown - ([a93ae4b](https://github.com/szinn/MailKeep/commit/a93ae4b323f4c1c6eecdef751ebfb28301ac4f24))
- _(imap)_ Self-manage greenmail via testcontainers - ([4a441fe](https://github.com/szinn/MailKeep/commit/4a441fe0776521db91e419b297e8f31ed17c1ab1))
- _(imap)_ Move greenmail integration tests to integration-tests crate - ([34cfa63](https://github.com/szinn/MailKeep/commit/34cfa63d1de4a3a572631eac5c226811a6ec3d1e))
- _(imap)_ Add gated greenmail integration tests - ([9675752](https://github.com/szinn/MailKeep/commit/96757522df098ca97f65bbc1d8c9f0bdf51ebb63))
- _(integration)_ MK-4 folder + message cascade and UIDVALIDITY tests - ([e150935](https://github.com/szinn/MailKeep/commit/e1509356bccf2514924cea52604b501c414b28e9))
- _(integration-tests)_ MK-2 acceptance tests for jobs subsystem - ([ce784b1](https://github.com/szinn/MailKeep/commit/ce784b1f2f7a2074e8006aeb4c489d6f5d904890))
- _(parser)_ Move fixtures out of tests/ into crate-level fixtures/ - ([7c8ff42](https://github.com/szinn/MailKeep/commit/7c8ff421e82b637d64bbdf0a6c4a22d0b62abbda))
- _(storage)_ Inline filesystem tests into the storage crate - ([4e8f6e3](https://github.com/szinn/MailKeep/commit/4e8f6e3849e756b352282d571f627edb151a02d1))

### Miscellaneous Tasks

- _(frontend)_ Resolve clippy warnings - ([eb2dfd3](https://github.com/szinn/MailKeep/commit/eb2dfd34011e6fde310e34444809e78bbe9714c1))
- Resolve non-frontend clippy warnings - ([da66de9](https://github.com/szinn/MailKeep/commit/da66de9ddb128184c7921df6fd0a237d68ccb839))
