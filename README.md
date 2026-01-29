# dbus-router

A dual-upstream D-Bus router library for sandboxed applications.

## Overview

`dbus-router` sits between sandboxed applications and D-Bus, routing messages to either a host bus or a sandbox bus based on configurable rules. This enables fine-grained control over which D-Bus services a sandboxed app can access on the host system.

## Architecture

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│  Sandbox App    │     │   dbus-router   │     │    Host Bus     │
│  (e.g. VSCode)  │────▶│                 │────▶│  (system/user)  │
└─────────────────┘     │   ┌─────────┐   │     └─────────────────┘
                        │   │ Routing │   │
                        │   │  Rules  │   │     ┌─────────────────┐
                        │   └─────────┘   │────▶│   Sandbox Bus   │
                        └─────────────────┘     │   (isolated)    │
                                                └─────────────────┘
```

## Sequence Diagram

```mermaid
sequenceDiagram
    participant App as Sandbox App
    participant Router as dbus-router
    participant Sandbox as Sandbox Bus
    participant Host as Host Bus
    participant HostClient as Host Client

    %% ========== Phase 1: Connection & Authentication ==========
    rect rgb(240, 248, 255)
        Note over App,Host: Phase 1: Connection & Authentication

        App->>Router: Connect

        Note right of Router: Auth passthrough to Sandbox
        App->>Router: AUTH EXTERNAL
        Router->>Sandbox: AUTH EXTERNAL
        Sandbox-->>Router: OK
        Router-->>App: OK
        App->>Router: BEGIN

        Note right of Router: Router authenticates with Host<br/>(Hello() only for non-hostpass clients)
        Router->>Host: AUTH EXTERNAL + BEGIN
        Router->>Host: Hello()
        Host-->>Router: unique name :1.x
    end

    %% ========== Phase 2: Default Routing (to Sandbox) ==========
    rect rgb(240, 255, 240)
        Note over App,Host: Phase 2: Default Routing → Sandbox

        App->>Router: Hello()
        Router->>Sandbox: Hello()
        Sandbox-->>Router: MethodReturn
        Router-->>App: MethodReturn

        App->>Router: Call org.example.Service
        Router->>Sandbox: Call org.example.Service
        Sandbox-->>Router: MethodReturn / Error
        Router-->>App: MethodReturn / Error
    end

    %% ========== Phase 3: Host Routes ==========
    rect rgb(255, 250, 240)
        Note over App,Host: Phase 3: Host Routes (config: org.fcitx.*)

        App->>Router: Call org.fcitx.Fcitx5
        Router->>Host: Call org.fcitx.Fcitx5
        Host-->>Router: MethodReturn
        Router-->>App: MethodReturn
    end

    %% ========== Phase 4: Hostpass Mode ==========
    rect rgb(255, 240, 245)
        Note over App,Host: Phase 4: Hostpass (config: */python3*)

        Note right of Router: Router skips Hello() on Host bus<br/>Client's Hello() will be forwarded
        App->>Router: Hello()
        Router->>Host: Hello()
        Host-->>Router: unique name :1.y
        Router-->>App: unique name :1.y

        Note right of Router: All messages route to Host
        App->>Router: RequestName("org.test.Svc")
        Router->>Host: RequestName("org.test.Svc")
        Host-->>Router: OK
        Router-->>App: OK

        Note over Host: Service visible on Host Bus

        HostClient->>Host: Call org.test.Svc
        Host->>Router: Call org.test.Svc
        Router->>App: Call org.test.Svc
        App-->>Router: MethodReturn
        Router-->>Host: MethodReturn
        Host-->>HostClient: MethodReturn

        App->>Router: Emit Signal
        Router->>Host: Signal
        Host->>HostClient: Signal
    end
```

## Configuration

Create a TOML configuration file:

```toml
# Routes specific destinations to host bus
[[host_routes]]
destination = "org.freedesktop.portal.*"

[[host_routes]]
destination = "org.fcitx.*"

# Allow specific processes to register services on host bus
[[hostpass]]
process = "*/python3*"

[[hostpass]]
process = "/usr/bin/my-service"
```

## Usage

### As a Library

```rust
use dbus_router::{Config, Router};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::load(&PathBuf::from("router.toml"))?;

    let router = Router::new(
        PathBuf::from("/run/user/1000/bus"),      // listen socket
        "unix:path=/run/user/1000/host".into(),   // host bus
        "unix:path=/run/user/1000/sandbox".into(), // sandbox bus
        config,
    );

    router.run().await
}
```

### As a Binary

```bash
dbus-router \
    --listen /run/user/1000/bus \
    --host "unix:path=/run/user/1000/host" \
    --sandbox "unix:path=/run/user/1000/sandbox" \
    --config router.toml
```

## Message Type Coverage

| Message Type   | Sandbox Routing | Host Routing | Hostpass |
|----------------|-----------------|--------------|----------|
| METHOD_CALL    | ✅              | ✅           | ✅       |
| METHOD_RETURN  | ✅              | ✅           | ✅       |
| ERROR          | ✅              | ✅           | ✅       |
| SIGNAL         | ✅              | ✅           | ✅       |

## License

MIT
