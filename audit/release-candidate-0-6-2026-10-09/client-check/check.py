
from pathlib import Path
from types import SimpleNamespace
import runpy
client = SimpleNamespace(**runpy.run_path(str(Path(__file__).with_name("instructions.py"))))

def refused(call):
    try:
        call()
    except (ValueError, OverflowError, TypeError, UnicodeError):
        return
    raise AssertionError('invalid input was accepted')

first = bytes([1, 1]) + (42).to_bytes(8, 'little')
second = bytes([1, 7]) + (99).to_bytes(8, 'little')
assert client.build_submit([first, second], 513) == bytes([0, 2, 0]) + first + second + bytes([1, 2])
for count in [0, 1, 4]:
    assert len(client.build_submit([first] * count, 513)) == 5 + 10 * count
refused(lambda: client.build_submit([first] * 5, 513))
for width in [0, 9, 11]:
    refused(lambda: client.build_submit([bytes(width)], 513))
assert client.build_bytes([b'*', b'c'], 'ok') == bytes([1, 2, 0, 42, 99, 2, 0, 111, 107])
assert client.build_aliases([b'\x07\x00', b'\x0b\x00'], 'é', b'\x01\x02', b'\x01\x02\x03') == bytes([2, 2, 0, 7, 0, 11, 0, 2, 0, 0xc3, 0xa9, 1, 2, 1, 2, 3])
refused(lambda: client.build_bytes([], 'é' * 9))
refused(lambda: client.build_bytes([], '\ud800'))
refused(lambda: client.build_aliases([], '', b'\x01', b'\x01\x02\x03'))
refused(lambda: client.build_aliases([], '', b'\x01\x02', b'\x01\x02\x03\x04'))
assert client.build_submit.ACCOUNT_ORDER == (('authority', {'writable': False, 'signer': True, 'layout': ''}),)
print('Generated Python client wire, alias, bounds, UTF-8, and privilege checks passed.')
