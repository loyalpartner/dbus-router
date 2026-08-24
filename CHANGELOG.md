# Changelog

## [0.11.0](https://github.com/loyalpartner/dbus-router/compare/v0.10.0...v0.11.0) (2026-08-24)


### Features

* forward unix fds through the router ([fa576ea](https://github.com/loyalpartner/dbus-router/commit/fa576eaca144c5b855d9295aa0a6520ecf6bc765))

## [0.10.0](https://github.com/loyalpartner/dbus-router/compare/v0.9.0...v0.10.0) (2026-01-31)


### Features

* dbus modules and routing ([6598b74](https://github.com/loyalpartner/dbus-router/commit/6598b747d473630993eb00a1ca6ca23b0c99b306))

## [0.9.0](https://github.com/loyalpartner/dbus-router/compare/v0.8.0...v0.9.0) (2026-01-31)


### Features

* add dbus-monitor style message logging ([b6a2da0](https://github.com/loyalpartner/dbus-router/commit/b6a2da0a7eb2a0acba8a5dd6fd53aff70d887603))


### Bug Fixes

* route GetConnectionSELinux and GetAdtAudit by fake unique name ([459c17c](https://github.com/loyalpartner/dbus-router/commit/459c17c7f224de246ee9e0224e46f4ed780e86fe))

## [0.8.0](https://github.com/loyalpartner/dbus-router/compare/v0.7.0...v0.8.0) (2026-01-30)


### Features

* add message context to header rewrite debug logs ([0f0582a](https://github.com/loyalpartner/dbus-router/commit/0f0582a5259b1af1d6e5c688db1bd105086174b5))


### Bug Fixes

* hostpass clients survive sandbox bus disconnection ([e687876](https://github.com/loyalpartner/dbus-router/commit/e6878768428a186c5106d8cbd7d9be74e247e72e))

## [0.7.0](https://github.com/loyalpartner/dbus-router/compare/v0.6.0...v0.7.0) (2026-01-30)


### Features

* improve header field parsing with better type support and debugging ([5e30ccf](https://github.com/loyalpartner/dbus-router/commit/5e30ccf3ff88ba4197b9153e2bc68eb29f67b303))

## [0.6.0](https://github.com/loyalpartner/dbus-router/compare/v0.5.2...v0.6.0) (2026-01-30)


### Features

* implement smart signal routing with fake unique name support ([#14](https://github.com/loyalpartner/dbus-router/issues/14)) ([972a8cf](https://github.com/loyalpartner/dbus-router/commit/972a8cf40b57577c1d3e6aa92b870591e7e6c32c))

## [0.5.2](https://github.com/loyalpartner/dbus-router/compare/v0.5.1...v0.5.2) (2026-01-29)


### Bug Fixes

* skip Hello() for hostpass clients to prevent duplicate Hello error ([96e5df9](https://github.com/loyalpartner/dbus-router/commit/96e5df9d990e31ea5bf0391269cff41b536a026e))

## [0.5.1](https://github.com/loyalpartner/dbus-router/compare/v0.5.0...v0.5.1) (2026-01-29)


### Bug Fixes

* consume NameAcquired signal after Hello() on host bus ([8590944](https://github.com/loyalpartner/dbus-router/commit/8590944fc646ef196c4ee95e6931b11e51e8d48e))

## [0.5.0](https://github.com/loyalpartner/dbus-router/compare/v0.4.3...v0.5.0) (2026-01-29)


### Features

* **router:** add comprehensive integration tests ([43577cc](https://github.com/loyalpartner/dbus-router/commit/43577cc753189cc9bbe1715469ece965cad1ea6a))
* **testing:** add comprehensive integration test framework ([43577cc](https://github.com/loyalpartner/dbus-router/commit/43577cc753189cc9bbe1715469ece965cad1ea6a))

## [0.4.3](https://github.com/loyalpartner/dbus-router/compare/v0.4.2...v0.4.3) (2026-01-29)


### Bug Fixes

* use zvariant for D-Bus header parsing and fix hostpass routing ([c423dd1](https://github.com/loyalpartner/dbus-router/commit/c423dd1d7df5ef259f2ae890fd7c8e33519d66bf))

## [0.4.2](https://github.com/loyalpartner/dbus-router/compare/v0.4.1...v0.4.2) (2026-01-29)


### Bug Fixes

* correctly skip unhandled header fields in parse_header_fields ([b2ad1a8](https://github.com/loyalpartner/dbus-router/commit/b2ad1a8dc22b5df09c034951fedb39f7d7f2fd5c))

## [0.4.1](https://github.com/loyalpartner/dbus-router/compare/v0.4.0...v0.4.1) (2026-01-29)


### Bug Fixes

* wait for NEGOTIATE_UNIX_FD response before completing auth ([1d61aec](https://github.com/loyalpartner/dbus-router/commit/1d61aec41b059e2a75ec1c8cd34e8259e240310f))

## [0.4.0](https://github.com/loyalpartner/dbus-router/compare/v0.3.2...v0.4.0) (2026-01-29)


### Features

* add hostpass for sandbox service export to host bus ([15ff494](https://github.com/loyalpartner/dbus-router/commit/15ff4940212c07cf51456d9ce35fcbee13a03759))

## [0.3.2](https://github.com/loyalpartner/dbus-router/compare/v0.3.1...v0.3.2) (2026-01-29)


### Bug Fixes

* add required crates.io metadata fields ([e76b250](https://github.com/loyalpartner/dbus-router/commit/e76b2504a9764d430a60cc412e67a8ba8dbafa4b))

## [0.3.1](https://github.com/loyalpartner/dbus-router/compare/v0.3.0...v0.3.1) (2026-01-29)


### Bug Fixes

* use correct release-please output names for root path ([328d68a](https://github.com/loyalpartner/dbus-router/commit/328d68a297ae069b2230d145f8c10420662a5005))

## [0.3.0](https://github.com/loyalpartner/dbus-router/compare/v0.2.1...v0.3.0) (2026-01-29)


### Features

* add workflow_dispatch to publish workflow ([db26e57](https://github.com/loyalpartner/dbus-router/commit/db26e57addd9aff1ef8ed4135163d0e510517644))


### Bug Fixes

* split release workflow to use release event trigger ([70e300a](https://github.com/loyalpartner/dbus-router/commit/70e300a037846eafa8021bc7dcd43d00efabf6a6))

## [0.2.1](https://github.com/loyalpartner/dbus-router/compare/v0.2.0...v0.2.1) (2026-01-29)


### Bug Fixes

* correct release-please output variable names ([230f92b](https://github.com/loyalpartner/dbus-router/commit/230f92b2ecd8375b0829d57ba5821a16aa32e8d7))

## [0.2.0](https://github.com/loyalpartner/dbus-router/compare/v0.1.0...v0.2.0) (2026-01-29)


### Features

* add automated release workflow with release-please ([b3e08dd](https://github.com/loyalpartner/dbus-router/commit/b3e08ddfd01d95afbedcfc8307c1a357fc08e486))
* initial release of dbus-router library ([6274c52](https://github.com/loyalpartner/dbus-router/commit/6274c523bbe52515e7851bc812b6ba3ef06553d2))
