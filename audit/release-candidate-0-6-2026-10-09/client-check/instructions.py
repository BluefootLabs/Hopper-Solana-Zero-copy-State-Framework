"""Instruction builders for program `hopper-lang`."""
import struct
import builtins as _hopper_builtins

def build_submit(orders: list[bytes], nonce: int) -> bytes:
    """Assemble the raw instruction data for `submit`. tag=0"""
    _hopper_args = (orders, nonce, )
    import builtins as _hopper_builtins
    import struct
    parts = [_hopper_builtins.bytes([0])]
    value = _hopper_args[0]
    if _hopper_builtins.len(value) > 4:
        raise _hopper_builtins.ValueError("orders exceeds capacity")
    if _hopper_builtins.any(not _hopper_builtins.isinstance(item, (_hopper_builtins.bytes, _hopper_builtins.bytearray)) or _hopper_builtins.len(item) != 10 for item in value):
        raise _hopper_builtins.ValueError("orders element width mismatch")
    parts.append(struct.pack("<H", _hopper_builtins.len(value)))
    parts.extend(_hopper_builtins.bytes(item) for item in value)
    value = _hopper_args[1]
    parts.append(struct.pack("<H", value))
    return b"".join(parts)

build_submit.ACCOUNT_ORDER = (
    ("authority", {"writable": False, "signer": True, "layout": ""}),
)

def build_bytes(bytes: list[bytes], note: str) -> bytes:
    """Assemble the raw instruction data for `bytes`. tag=1"""
    _hopper_args = (bytes, note, )
    import builtins as _hopper_builtins
    import struct
    parts = [_hopper_builtins.bytes([1])]
    value = _hopper_args[0]
    if _hopper_builtins.len(value) > 8:
        raise _hopper_builtins.ValueError("bytes exceeds capacity")
    if _hopper_builtins.any(not _hopper_builtins.isinstance(item, (_hopper_builtins.bytes, _hopper_builtins.bytearray)) or _hopper_builtins.len(item) != 1 for item in value):
        raise _hopper_builtins.ValueError("bytes element width mismatch")
    parts.append(struct.pack("<H", _hopper_builtins.len(value)))
    parts.extend(_hopper_builtins.bytes(item) for item in value)
    value = _hopper_args[1]
    encoded = value.encode("utf-8", errors="strict")
    if _hopper_builtins.len(encoded) > 16:
        raise _hopper_builtins.ValueError("note exceeds byte capacity")
    parts.append(struct.pack("<H", _hopper_builtins.len(encoded)))
    parts.append(encoded)
    return b"".join(parts)

build_bytes.ACCOUNT_ORDER = (
    ("authority", {"writable": False, "signer": True, "layout": ""}),
)

def build_aliases(quantities: list[bytes], label: str, nonce: bytes, salt: bytes) -> bytes:
    """Assemble the raw instruction data for `aliases`. tag=2"""
    _hopper_args = (quantities, label, nonce, salt, )
    import builtins as _hopper_builtins
    import struct
    parts = [_hopper_builtins.bytes([2])]
    value = _hopper_args[0]
    if _hopper_builtins.len(value) > 4:
        raise _hopper_builtins.ValueError("quantities exceeds capacity")
    if _hopper_builtins.any(not _hopper_builtins.isinstance(item, (_hopper_builtins.bytes, _hopper_builtins.bytearray)) or _hopper_builtins.len(item) != 2 for item in value):
        raise _hopper_builtins.ValueError("quantities element width mismatch")
    parts.append(struct.pack("<H", _hopper_builtins.len(value)))
    parts.extend(_hopper_builtins.bytes(item) for item in value)
    value = _hopper_args[1]
    encoded = value.encode("utf-8", errors="strict")
    if _hopper_builtins.len(encoded) > 16:
        raise _hopper_builtins.ValueError("label exceeds byte capacity")
    parts.append(struct.pack("<H", _hopper_builtins.len(encoded)))
    parts.append(encoded)
    value = _hopper_args[2]
    if not _hopper_builtins.isinstance(value, (_hopper_builtins.bytes, _hopper_builtins.bytearray)) or _hopper_builtins.len(value) != 2:
        raise _hopper_builtins.ValueError("nonce width mismatch")
    parts.append(struct.pack("<2s", value))
    value = _hopper_args[3]
    if not _hopper_builtins.isinstance(value, (_hopper_builtins.bytes, _hopper_builtins.bytearray)) or _hopper_builtins.len(value) != 3:
        raise _hopper_builtins.ValueError("salt width mismatch")
    parts.append(struct.pack("<3s", value))
    return b"".join(parts)

build_aliases.ACCOUNT_ORDER = (
    ("authority", {"writable": False, "signer": True, "layout": ""}),
)

