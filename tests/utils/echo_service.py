#!/usr/bin/env python3
"""Simple D-Bus Echo service for testing hostpass routing."""

import asyncio
from dbus_next.aio import MessageBus
from dbus_next.service import ServiceInterface, method, signal
from dbus_next import DBusError


class EchoInterface(ServiceInterface):
    def __init__(self):
        super().__init__('org.test.Echo')

    @method()
    def Echo(self, message: 's') -> 's':
        return message

    @method()
    def Ping(self) -> 's':
        return 'pong'

    @signal()
    def TestSignal(self, message: 's') -> 's':
        return message

    @method()
    def EmitSignal(self, message: 's') -> 's':
        """Emit TestSignal and return confirmation."""
        self.TestSignal(message)
        return 'emitted'

    @method()
    def RaiseError(self, error_message: 's') -> 's':
        """Raise a D-Bus error for testing ERROR message routing."""
        raise DBusError('org.test.Echo.TestError', error_message)


async def main():
    bus = await MessageBus().connect()
    interface = EchoInterface()
    bus.export('/org/test/Echo', interface)
    await bus.request_name('org.test.Echo')
    await bus.wait_for_disconnect()


if __name__ == '__main__':
    asyncio.run(main())
