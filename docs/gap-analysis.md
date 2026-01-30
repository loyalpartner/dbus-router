# D-Bus Router Gap Analysis

This document compares the current hsdbus implementation against the D-Bus specification
and identifies missing features that should have integration tests.

**Last Updated:** 2026-01-30

## Current Implementation Status

### org.freedesktop.DBus Methods

| Method | Implemented | Tested | Notes |
|--------|-------------|--------|-------|
| Hello | Yes | Yes | Returns unique name with :s./:h. prefix |
| RequestName | Yes | Yes | Tracks sandbox_services for reverse proxy |
| ReleaseName | Yes | Yes | Routing and signal handling verified |
| ListNames | Yes | Yes | Merges both buses with fake prefixes |
| ListActivatableNames | Yes | Yes | Same merge logic as ListNames |
| NameHasOwner | Yes | Yes | Routes based on name prefix (see limitations) |
| GetNameOwner | Yes | Yes | Rewrites unique name in response |
| StartServiceByName | Yes | Yes | Routes based on service name |
| ListQueuedOwners | Routing only | No | Returns unique names - need rewrite |
| AddMatch | Yes | Yes | Rewrites sender in match rule |
| RemoveMatch | Yes | Yes | Same as AddMatch |
| GetConnectionUnixUser | Yes | Yes | Rewrites unique name in request |
| GetConnectionUnixProcessID | Yes | Yes | Rewrites unique name in request |
| GetConnectionCredentials | Yes | Yes | Rewrites unique name in request |
| GetId | Passthrough | Yes | No special handling needed |
| UpdateActivationEnvironment | No | No | Probably not needed for sandbox |
| GetAdtAuditSessionData | No | No | Solaris-specific |
| GetConnectionSELinuxSecurityContext | No | No | May not be needed |

### org.freedesktop.DBus Signals

| Signal | Implemented | Tested | Notes |
|--------|-------------|--------|-------|
| NameOwnerChanged | Rewrite | Yes | Rewrites unique names in body |
| NameAcquired | Passthrough | Yes | Tested during Hello |
| NameLost | Passthrough | Yes | Tested in ReleaseName tests |
| ActivatableServicesChanged | No | No | Probably not relevant |

### Standard Interfaces

| Interface | Method | Implemented | Tested | Notes |
|-----------|--------|-------------|--------|-------|
| Peer | Ping | Passthrough | Yes | Used in host_route tests |
| Peer | GetMachineId | Passthrough | No | May need consistent response |
| Introspectable | Introspect | Passthrough | Yes | Tested in sandbox_routing |
| Properties | Get/Set/GetAll | Passthrough | No | May contain unique names |
| ObjectManager | GetManagedObjects | Passthrough | No | May contain unique names |
| ObjectManager | InterfacesAdded | Passthrough | Yes | Signals work |
| ObjectManager | InterfacesRemoved | Passthrough | No | Need test |

### Match Rule Features

| Feature | Implemented | Tested | Notes |
|---------|-------------|--------|-------|
| type | Yes | Yes | Match by message type |
| sender | Yes | Yes | Rewrite fake name to real |
| interface | Yes | Yes | Standard match |
| member | Yes | Yes | Standard match |
| path | Yes | No | Standard match |
| path_namespace | Unknown | No | Prefix matching |
| destination | Yes | No | Standard match |
| argN | Unknown | No | Argument content matching |
| arg0namespace | Unknown | No | Namespace prefix matching |
| eavesdrop | Unknown | No | Security implications |

### Header Flags

| Flag | Implemented | Tested | Notes |
|------|-------------|--------|-------|
| NO_REPLY_EXPECTED | Unknown | No | Skip waiting for reply |
| NO_AUTO_START | Unknown | No | Don't activate service |
| ALLOW_INTERACTIVE_AUTHORIZATION | Unknown | No | PolicyKit support |

## Integration Test Status

### Completed Tests

1. **test_release_name.py** - ReleaseName behavior
   - Basic release
   - NameLost signal reception
   - NameOwnerChanged signal with empty new_owner
   - Error for releasing unowned name

2. **test_list_activatable_names.py** - ListActivatableNames
   - Basic functionality
   - Merged results from both buses
   - No duplicate entries

3. **test_name_has_owner.py** - NameHasOwner
   - Well-known names (owned/not owned)
   - Sandbox services
   - Host-routed names

4. **test_remove_match.py** - RemoveMatch
   - Basic add/remove
   - Error for non-existent rule
   - Signal filtering stops after remove
   - Sender rewriting

5. **test_get_connection_credentials.py** - Credential queries
   - GetConnectionUnixUser
   - GetConnectionUnixProcessID
   - GetConnectionCredentials
   - Fake name rewriting
   - Error for invalid names

6. **test_start_service_by_name.py** - StartServiceByName
   - Non-activatable services (error)
   - org.freedesktop.DBus (already running)
   - Host-routed services

### Remaining Tests to Implement

**Priority 2: Edge Cases**

1. **test_list_queued_owners.py** - ListQueuedOwners
   - Multiple clients request same name
   - Verify queued owners have correct prefix

2. **test_match_rule_path.py** - Test path matching
   - Add match with specific path
   - Verify signal filtering

3. **test_no_reply_expected.py** - Test NO_REPLY_EXPECTED flag
   - Send method call with flag
   - Verify no reply is sent

**Priority 3: Robustness**

4. **test_invalid_match_rule.py** - Invalid match rule handling

5. **test_connection_timeout.py** - Connection handling edge cases

6. **test_large_message.py** - Message size limits

## Known Limitations

### NameHasOwner with Fake Unique Names

`NameHasOwner` for fake unique names (e.g., `:s.1.0`) doesn't work correctly.
The router routes to the correct bus but doesn't rewrite the body argument,
so the underlying bus returns False (doesn't recognize the fake name).

**Workaround:** Use well-known names for NameHasOwner queries.

**Fix:** Add `NameHasOwner` to `METHODS_NEED_REQUEST_REWRITE` in `dbus_daemon.rs`.

### Credential Queries Return Proxy Credentials

When querying `GetConnectionUnixProcessID` through the router, the returned PID
is the router's PID (or the connection's PID from the bus's perspective), not
the original client's PID. This is inherent to proxy architecture.

The UID is typically consistent since all processes run as the same user.

## Implementation Notes

### ReleaseName Handling
Currently tracks `sandbox_services` on RequestName. Need to:
1. Parse ReleaseName to get service name
2. Remove from sandbox_services on success
3. Handle case where service was on host bus

### ListQueuedOwners Response Rewriting
Response is `ARRAY STRING` of unique names. Need to:
1. Parse response body
2. Add appropriate prefix (:h. or :s.) based on bus
3. Rebuild message with rewritten body

### GetConnectionCredentials Routing
Currently routes based on fake name prefix. Need to verify:
1. :h.X.Y routes to host with :X.Y
2. :s.X.Y routes to sandbox with :X.Y
3. Error for names without prefix

### Match Rule edge cases
Need to handle:
1. Match rules without sender (broadcast)
2. Match rules with path_namespace
3. Match rules with argN arguments
4. Eavesdrop rules (should probably be blocked)

## Testing Strategy

Each test should:
1. Set up host and sandbox dbus-daemon instances
2. Start the router with appropriate config
3. Connect client(s) through router
4. Execute the specific scenario
5. Verify correct behavior
6. Check router logs for errors

Tests should be independent and not rely on state from other tests.
