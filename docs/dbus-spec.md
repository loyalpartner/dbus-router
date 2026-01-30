# D-Bus Specification Reference

This document summarizes the D-Bus Specification (version 0.43) from
https://dbus.freedesktop.org/doc/dbus-specification.html

## Overview

D-Bus is a system for low-overhead interprocess communication (IPC). It provides:
- A type system for serializing data
- A message protocol for sending method calls, replies, errors, and signals
- A bus daemon for routing messages between applications

## Message Types

| Type | Value | Description |
|------|-------|-------------|
| METHOD_CALL | 1 | Request to invoke a method |
| METHOD_RETURN | 2 | Response to a method call |
| ERROR | 3 | Error response to a method call |
| SIGNAL | 4 | Broadcast notification |

## Header Fields

| Code | Name | Type | Description |
|------|------|------|-------------|
| 1 | PATH | OBJECT_PATH | Object path for method call |
| 2 | INTERFACE | STRING | Interface name |
| 3 | MEMBER | STRING | Method or signal name |
| 4 | ERROR_NAME | STRING | Error name for ERROR messages |
| 5 | REPLY_SERIAL | UINT32 | Serial number of the call being replied to |
| 6 | DESTINATION | STRING | Name of the connection to receive this message |
| 7 | SENDER | STRING | Unique name of the sending connection |
| 8 | SIGNATURE | SIGNATURE | Body type signature |
| 9 | UNIX_FDS | UINT32 | Number of Unix file descriptors |

## Header Flags

| Flag | Value | Description |
|------|-------|-------------|
| NO_REPLY_EXPECTED | 0x1 | No METHOD_RETURN or ERROR expected |
| NO_AUTO_START | 0x2 | Don't auto-start the service |
| ALLOW_INTERACTIVE_AUTHORIZATION | 0x4 | Allow interactive authorization |

## org.freedesktop.DBus Interface Methods

### Hello
```
STRING Hello()
```
Must be the first method called. Returns the unique name assigned to the connection.

### RequestName
```
UINT32 RequestName(STRING name, UINT32 flags)
```
Request ownership of a well-known name.

**Flags:**
- `ALLOW_REPLACEMENT` (0x1) - Allow other connections to take the name
- `REPLACE_EXISTING` (0x2) - Replace existing owner
- `DO_NOT_QUEUE` (0x4) - Don't queue if name is owned

**Return values:**
- `PRIMARY_OWNER` (1) - Name ownership acquired
- `IN_QUEUE` (2) - Queued behind existing owner
- `EXISTS` (3) - Name already owned, not queued
- `ALREADY_OWNER` (4) - Already own the name

### ReleaseName
```
UINT32 ReleaseName(STRING name)
```
Release ownership of a well-known name.

**Return values:**
- `RELEASED` (1) - Name was released
- `NON_EXISTENT` (2) - Name doesn't exist
- `NOT_OWNER` (3) - Not the owner

### ListNames
```
ARRAY STRING ListNames()
```
Returns list of all currently-owned names (unique and well-known).

### ListActivatableNames
```
ARRAY STRING ListActivatableNames()
```
Returns list of names that can be activated.

### NameHasOwner
```
BOOLEAN NameHasOwner(STRING name)
```
Checks whether a name has an owner.

### GetNameOwner
```
STRING GetNameOwner(STRING name)
```
Returns the unique connection name of the primary owner.

### StartServiceByName
```
UINT32 StartServiceByName(STRING name, UINT32 flags)
```
Try to launch the service associated with a name.

**Return values:**
- `SUCCESS` (1) - Service started successfully
- `ALREADY_RUNNING` (2) - Service was already running

### ListQueuedOwners
```
ARRAY STRING ListQueuedOwners(STRING name)
```
Returns the unique connection names of connections queued for the name.

### AddMatch
```
VOID AddMatch(STRING rule)
```
Adds a match rule to receive signals.

**Match rule keys:**
- `type` - Message type (signal, method_call, method_return, error)
- `sender` - Sender name
- `interface` - Interface name
- `member` - Method or signal name
- `path` - Object path (exact match)
- `path_namespace` - Object path namespace (prefix match)
- `destination` - Destination name
- `arg0`, `arg1`, ... - Argument content
- `arg0namespace` - Namespace match for arg0
- `eavesdrop` - Receive messages not addressed to us

### RemoveMatch
```
VOID RemoveMatch(STRING rule)
```
Removes a match rule.

### GetConnectionUnixUser
```
UINT32 GetConnectionUnixUser(STRING connection_name)
```
Returns the Unix UID of the connection.

### GetConnectionUnixProcessID
```
UINT32 GetConnectionUnixProcessID(STRING connection_name)
```
Returns the Unix PID of the connection.

### GetConnectionCredentials
```
ARRAY DICT<STRING,VARIANT> GetConnectionCredentials(STRING connection_name)
```
Returns credentials of the connection.

**Possible keys:**
- `UnixUserID` (UINT32)
- `UnixGroupIDs` (ARRAY UINT32)
- `ProcessID` (UINT32)
- `LinuxSecurityLabel` (ARRAY BYTE)
- `WindowsSID` (STRING)

### GetId
```
STRING GetId()
```
Returns a unique ID for the bus.

### UpdateActivationEnvironment
```
VOID UpdateActivationEnvironment(ARRAY DICT<STRING,STRING> environment)
```
Updates the environment used for service activation.

## org.freedesktop.DBus Signals

### NameOwnerChanged
```
NameOwnerChanged(STRING name, STRING old_owner, STRING new_owner)
```
Sent when a name's owner changes.

### NameLost
```
NameLost(STRING name)
```
Sent to the previous owner when they lose a name.

### NameAcquired
```
NameAcquired(STRING name)
```
Sent to the new owner when they acquire a name.

### ActivatableServicesChanged
```
ActivatableServicesChanged()
```
Sent when the list of activatable services changes.

## Standard Interfaces

### org.freedesktop.DBus.Peer
```
Ping()
STRING GetMachineId()
```

### org.freedesktop.DBus.Introspectable
```
STRING Introspect()
```

### org.freedesktop.DBus.Properties
```
VARIANT Get(STRING interface, STRING property)
VOID Set(STRING interface, STRING property, VARIANT value)
ARRAY DICT<STRING,VARIANT> GetAll(STRING interface)
SIGNAL PropertiesChanged(STRING interface, DICT<STRING,VARIANT> changed, ARRAY STRING invalidated)
```

### org.freedesktop.DBus.ObjectManager
```
ARRAY DICT<OBJPATH,DICT<STRING,DICT<STRING,VARIANT>>> GetManagedObjects()
SIGNAL InterfacesAdded(OBJPATH path, DICT<STRING,DICT<STRING,VARIANT>> interfaces)
SIGNAL InterfacesRemoved(OBJPATH path, ARRAY STRING interfaces)
```

## Wire Format

### Message Structure
1. Fixed header (12 bytes): endianness, type, flags, version, body length, serial
2. Header fields array length (4 bytes)
3. Header fields (variable, 8-byte aligned structs)
4. Padding to 8-byte boundary
5. Body

### Type Signatures
| Code | Type | Description |
|------|------|-------------|
| y | BYTE | Unsigned 8-bit |
| b | BOOLEAN | 32-bit, valid values 0 or 1 |
| n | INT16 | Signed 16-bit |
| q | UINT16 | Unsigned 16-bit |
| i | INT32 | Signed 32-bit |
| u | UINT32 | Unsigned 32-bit |
| x | INT64 | Signed 64-bit |
| t | UINT64 | Unsigned 64-bit |
| d | DOUBLE | IEEE 754 double |
| s | STRING | UTF-8 string |
| o | OBJECT_PATH | Valid object path |
| g | SIGNATURE | Type signature |
| a | ARRAY | Array of elements |
| ( | STRUCT | Structure start |
| ) | STRUCT | Structure end |
| v | VARIANT | Type in signature + value |
| { | DICT_ENTRY | Dictionary entry start |
| } | DICT_ENTRY | Dictionary entry end |
| h | UNIX_FD | Unix file descriptor |

## Authentication

D-Bus uses a line-based protocol for authentication before message exchange.

### Commands
- `AUTH [mechanism [initial-response]]`
- `CANCEL`
- `BEGIN`
- `DATA <base64>`
- `ERROR [message]`
- `NEGOTIATE_UNIX_FD`
- `AGREE_UNIX_FD`
- `REJECTED <mechanisms>`
- `OK <server-guid>`

### SASL Mechanisms
- `EXTERNAL` - Use out-of-band credentials (Unix socket)
- `DBUS_COOKIE_SHA1` - Cookie-based authentication
- `ANONYMOUS` - Anonymous connection

## Error Names

Common error names:
- `org.freedesktop.DBus.Error.Failed`
- `org.freedesktop.DBus.Error.NoMemory`
- `org.freedesktop.DBus.Error.ServiceUnknown`
- `org.freedesktop.DBus.Error.NameHasNoOwner`
- `org.freedesktop.DBus.Error.NoReply`
- `org.freedesktop.DBus.Error.IOError`
- `org.freedesktop.DBus.Error.BadAddress`
- `org.freedesktop.DBus.Error.NotSupported`
- `org.freedesktop.DBus.Error.LimitsExceeded`
- `org.freedesktop.DBus.Error.AccessDenied`
- `org.freedesktop.DBus.Error.AuthFailed`
- `org.freedesktop.DBus.Error.NoServer`
- `org.freedesktop.DBus.Error.Timeout`
- `org.freedesktop.DBus.Error.NoNetwork`
- `org.freedesktop.DBus.Error.AddressInUse`
- `org.freedesktop.DBus.Error.Disconnected`
- `org.freedesktop.DBus.Error.InvalidArgs`
- `org.freedesktop.DBus.Error.FileNotFound`
- `org.freedesktop.DBus.Error.FileExists`
- `org.freedesktop.DBus.Error.UnknownMethod`
- `org.freedesktop.DBus.Error.UnknownObject`
- `org.freedesktop.DBus.Error.UnknownInterface`
- `org.freedesktop.DBus.Error.UnknownProperty`
- `org.freedesktop.DBus.Error.PropertyReadOnly`
- `org.freedesktop.DBus.Error.MatchRuleNotFound`
- `org.freedesktop.DBus.Error.MatchRuleInvalid`
- `org.freedesktop.DBus.Error.InteractiveAuthorizationRequired`

## Transport Addresses

Format: `transport:key=value,key=value,...`

### Unix Domain Sockets
- `unix:path=/path/to/socket`
- `unix:abstract=/abstract/name` (Linux only)
- `unix:tmpdir=/tmp` (auto-generated path)

### TCP
- `tcp:host=hostname,port=port`
- `tcp:host=hostname,port=port,family=ipv4`

### systemd
- `systemd:` (socket activation)

### launchd (macOS)
- `launchd:env=ENVIRONMENT_VARIABLE`

## References

- Full specification: https://dbus.freedesktop.org/doc/dbus-specification.html
- Protocol frozen: November 8, 2006
- Version: 0.43
