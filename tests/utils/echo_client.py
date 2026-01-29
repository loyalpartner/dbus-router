"""D-Bus client utilities for testing."""

import asyncio
from dbus_next.aio import MessageBus
from dbus_next import MessageType, DBusError


async def call_echo(dbus_addr: str, message: str) -> str:
    """Call Echo method on org.test.Echo service."""
    bus = await MessageBus(bus_address=dbus_addr).connect()
    introspection = await bus.introspect('org.test.Echo', '/org/test/Echo')
    proxy = bus.get_proxy_object('org.test.Echo', '/org/test/Echo', introspection)
    interface = proxy.get_interface('org.test.Echo')
    result = await interface.call_echo(message)
    bus.disconnect()
    return result


def sync_call_echo(dbus_addr: str, message: str) -> str:
    """Synchronous wrapper for call_echo."""
    return asyncio.run(call_echo(dbus_addr, message))


async def call_emit_signal(dbus_addr: str, message: str) -> str:
    """Call EmitSignal method on org.test.Echo service."""
    bus = await MessageBus(bus_address=dbus_addr).connect()
    introspection = await bus.introspect('org.test.Echo', '/org/test/Echo')
    proxy = bus.get_proxy_object('org.test.Echo', '/org/test/Echo', introspection)
    interface = proxy.get_interface('org.test.Echo')
    result = await interface.call_emit_signal(message)
    bus.disconnect()
    return result


def sync_call_emit_signal(dbus_addr: str, message: str) -> str:
    """Synchronous wrapper for call_emit_signal."""
    return asyncio.run(call_emit_signal(dbus_addr, message))


async def wait_for_signal(dbus_addr: str, timeout: float = 5.0) -> str | None:
    """Wait for TestSignal from org.test.Echo service.

    Registers a match rule with the bus and waits for the signal.
    Returns the signal message body or None if timeout.
    """
    from dbus_next import Message

    bus = await MessageBus(bus_address=dbus_addr).connect()
    received = asyncio.Event()
    message_body = None

    def on_message(msg):
        nonlocal message_body
        if (msg.message_type == MessageType.SIGNAL and
            msg.interface == 'org.test.Echo' and
            msg.member == 'TestSignal'):
            message_body = msg.body[0] if msg.body else None
            received.set()

    bus.add_message_handler(on_message)

    # Register match rule to receive signals
    await bus.call(
        Message(
            destination='org.freedesktop.DBus',
            path='/org/freedesktop/DBus',
            interface='org.freedesktop.DBus',
            member='AddMatch',
            signature='s',
            body=["type='signal',interface='org.test.Echo',member='TestSignal'"]
        )
    )

    try:
        await asyncio.wait_for(received.wait(), timeout)
    except asyncio.TimeoutError:
        pass

    bus.disconnect()
    return message_body


def sync_wait_for_signal(dbus_addr: str, timeout: float = 5.0) -> str | None:
    """Synchronous wrapper for wait_for_signal."""
    return asyncio.run(wait_for_signal(dbus_addr, timeout))


def emit_and_wait_for_signal(
    emit_addr: str, listen_addr: str, message: str, timeout: float = 5.0
) -> str | None:
    """Emit a signal via emit_addr and wait to receive it on listen_addr.

    This handles the threading coordination needed for signal tests.
    Returns the received signal message or None if timeout.
    """
    import threading

    signal_received = None
    listener_ready = threading.Event()
    listener_done = threading.Event()

    def listen_thread():
        nonlocal signal_received
        loop = asyncio.new_event_loop()
        asyncio.set_event_loop(loop)

        async def listen():
            nonlocal signal_received
            from dbus_next import Message

            bus = await MessageBus(bus_address=listen_addr).connect()
            received = asyncio.Event()

            def on_message(msg):
                nonlocal signal_received
                if (msg.message_type == MessageType.SIGNAL and
                    msg.interface == 'org.test.Echo' and
                    msg.member == 'TestSignal'):
                    signal_received = msg.body[0] if msg.body else None
                    received.set()

            bus.add_message_handler(on_message)

            await bus.call(
                Message(
                    destination='org.freedesktop.DBus',
                    path='/org/freedesktop/DBus',
                    interface='org.freedesktop.DBus',
                    member='AddMatch',
                    signature='s',
                    body=["type='signal',interface='org.test.Echo',member='TestSignal'"]
                )
            )
            listener_ready.set()

            try:
                await asyncio.wait_for(received.wait(), timeout)
            except asyncio.TimeoutError:
                pass
            bus.disconnect()

        loop.run_until_complete(listen())
        listener_done.set()

    thread = threading.Thread(target=listen_thread)
    thread.start()
    listener_ready.wait(timeout=2.0)

    # Emit signal
    result = sync_call_emit_signal(emit_addr, message)
    assert result == "emitted"

    listener_done.wait(timeout=timeout)
    thread.join(timeout=1.0)

    return signal_received


async def call_raise_error(dbus_addr: str, error_message: str) -> tuple[str, str]:
    """Call RaiseError method on org.test.Echo service.

    Returns tuple of (error_name, error_message) if error is raised,
    or raises an exception if no error is raised.
    """
    bus = await MessageBus(bus_address=dbus_addr).connect()
    introspection = await bus.introspect('org.test.Echo', '/org/test/Echo')
    proxy = bus.get_proxy_object('org.test.Echo', '/org/test/Echo', introspection)
    interface = proxy.get_interface('org.test.Echo')
    try:
        await interface.call_raise_error(error_message)
        bus.disconnect()
        raise AssertionError("Expected DBusError but method returned normally")
    except DBusError as e:
        bus.disconnect()
        return (e.type, e.text)


def sync_call_raise_error(dbus_addr: str, error_message: str) -> tuple[str, str]:
    """Synchronous wrapper for call_raise_error."""
    return asyncio.run(call_raise_error(dbus_addr, error_message))
