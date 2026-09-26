"""Darktide (Stingray) format-8 bundle chunks: detect and Oodle-compress stored chunks.

A format-8 bundle holds its data in 512 KiB chunks. The game's own bundles
store every chunk Oodle-compressed. A chunk exactly 512 KiB long is "stored"
(uncompressed): the regular bundle reader accepts that, but the game's
DirectStorage reader hands every chunk to Oodle and crashes with
"Failed to decompress ... from package". Replacement bundles must therefore
be compressed like the stock ones before they ship.

Oodle is not redistributable. `Oodle` loads the copy installed with the game
(`binaries/oo2core_9_win64.dll`) and only works on Windows (or Wine).
"""

from __future__ import annotations

import ctypes
import struct
from dataclasses import dataclass
from pathlib import Path

MAGIC = bytes.fromhex("080000f003000000")
CHUNK = 0x80000
KRAKEN = 8
LEVEL_NORMAL = 4
MAX_CHUNKS = 4096
MAX_INDEX = 65536


class BundleError(Exception):
    pass


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise BundleError(message)


@dataclass
class Layout:
    prefix: bytes          # magic, count, 256-byte block, index
    sizes: list[int]
    table_padding: bytes
    length: int
    chunk_padding: list[bytes]
    blocks: list[bytes]


def is_bundle(data: bytes) -> bool:
    return data[:8] == MAGIC


def parse(data: bytes) -> Layout:
    _require(is_bundle(data), "not a format-8 bundle")
    pos = 8

    def take(size: int) -> bytes:
        nonlocal pos
        _require(0 <= size <= len(data) - pos, "truncated bundle")
        out = data[pos:pos + size]
        pos += size
        return out

    def u32() -> int:
        return struct.unpack("<I", take(4))[0]

    count = u32()
    _require(1 <= count <= MAX_INDEX, "index count out of range")
    take(256)
    take(20 * count)
    prefix = data[:pos]
    chunks = u32()
    _require(1 <= chunks <= MAX_CHUNKS, "chunk count out of range")
    sizes = [u32() for _ in range(chunks)]
    _require(all(0 < s <= CHUNK for s in sizes), "chunk size out of range")
    table_padding = take((-pos) % 16)
    length, zero = u32(), u32()
    _require(zero == 0, "nonzero reserved length field")
    _require(0 < length <= chunks * CHUNK and (length + CHUNK - 1) // CHUNK == chunks,
             "logical length does not match chunk count")
    pads, blocks = [], []
    for size in sizes:
        _require(u32() == size, "chunk table and inline size differ")
        pads.append(take((-pos) % 16))
        blocks.append(take(size))
    _require(pos == len(data), "trailing bytes after last chunk")
    return Layout(prefix, sizes, table_padding, length, pads, blocks)


def stored_chunks(data: bytes) -> int:
    """Number of stored (uncompressed) chunks; 0 for non-bundles."""
    if not is_bundle(data):
        return 0
    return sum(1 for size in parse(data).sizes if size == CHUNK)


def build(layout: Layout, blocks: list[bytes]) -> bytes:
    out = bytearray(layout.prefix)
    out += struct.pack("<I", len(blocks))
    for block in blocks:
        out += struct.pack("<I", len(block))
    pad = (-len(out)) % 16
    if len(layout.table_padding) == pad:
        out += layout.table_padding
    else:
        _require(not any(layout.table_padding), "cannot move nonzero alignment padding")
        out += bytes(pad)
    out += struct.pack("<II", layout.length, 0)
    for i, block in enumerate(blocks):
        out += struct.pack("<I", len(block))
        pad = (-len(out)) % 16
        original = layout.chunk_padding[i]
        if len(original) == pad:
            out += original
        else:
            _require(not any(original), "cannot move nonzero alignment padding")
            out += bytes(pad)
        out += block
    return bytes(out)


class Oodle:
    """The game's own Oodle library, loaded read-only through ctypes."""

    def __init__(self, dll: Path):
        if not hasattr(ctypes, "WinDLL"):
            raise BundleError("Oodle compression needs Windows (the game's oo2core_9_win64.dll)")
        lib = ctypes.CDLL(str(dll))
        self._compress = lib.OodleLZ_Compress
        self._compress.restype = ctypes.c_int64
        self._compress.argtypes = [
            ctypes.c_int, ctypes.c_void_p, ctypes.c_int64, ctypes.c_void_p, ctypes.c_int,
            ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_int64,
        ]
        self._decompress = lib.OodleLZ_Decompress
        self._decompress.restype = ctypes.c_int64
        self._decompress.argtypes = [
            ctypes.c_void_p, ctypes.c_int64, ctypes.c_void_p, ctypes.c_int64, ctypes.c_int,
            ctypes.c_int, ctypes.c_int, ctypes.c_void_p, ctypes.c_int64, ctypes.c_void_p,
            ctypes.c_void_p, ctypes.c_void_p, ctypes.c_int64, ctypes.c_int,
        ]

    def compress(self, raw: bytes, level: int = LEVEL_NORMAL) -> bytes:
        src = ctypes.create_string_buffer(raw, len(raw))
        cap = len(raw) + 0x10000
        dst = ctypes.create_string_buffer(cap)
        n = self._compress(KRAKEN, src, len(raw), dst, level, None, None, None, None, 0)
        _require(0 < n <= cap, "Oodle compression failed")
        return dst.raw[:n]

    def decompress(self, block: bytes, size: int = CHUNK) -> bytes:
        src = ctypes.create_string_buffer(block, len(block))
        dst = ctypes.create_string_buffer(size)
        # Same call shape as the game: fuzz-safe, no CRC, unthreaded.
        n = self._decompress(src, len(block), dst, size, 1, 0, 0, None, 0, None, None, None, 0, 3)
        _require(n == size, "Oodle decompression failed")
        return dst.raw[:n]


def pack(data: bytes, oodle, level: int = LEVEL_NORMAL) -> bytes:
    """Compress every stored chunk; compressed chunks stay byte-identical.

    Every new chunk is decompressed again and compared with the original
    bytes, the result is re-parsed, and the full logical stream is checked.
    """
    layout = parse(data)
    blocks = []
    for block in layout.blocks:
        if len(block) != CHUNK:
            blocks.append(block)
            continue
        packed = oodle.compress(block, level)
        _require(len(packed) < CHUNK, "chunk did not compress below 512 KiB")
        _require(oodle.decompress(packed) == block, "Oodle round trip differs")
        blocks.append(packed)
    result = build(layout, blocks)
    again = parse(result)
    _require(again.prefix == layout.prefix and again.length == layout.length, "rebuilt header differs")
    _require(all(size < CHUNK for size in again.sizes), "stored chunks remain")
    before = b"".join(b if len(b) == CHUNK else oodle.decompress(b) for b in layout.blocks)
    after = b"".join(oodle.decompress(b) for b in again.blocks)
    _require(before == after, "logical contents changed")
    return result
