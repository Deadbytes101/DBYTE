from __future__ import annotations

import argparse
import shutil
import struct
from pathlib import Path

SECTOR = 512
TOTAL_SECTORS = 32768
SUPER_LBA = 256
DIR_LBA = 257
DIR_SECTORS = 4
BITMAP_LBA = 261
BITMAP_SECTORS = 8
DATA_LBA = 269
MIRROR_LBA = TOTAL_SECTORS - 1
MAGIC = 0x31504F48
MAX_FILE_SECTORS = 384
RECORD_NAME = b"DBYRT641"
USER_RECORD_NAME = b"USER.ELF"
DATA_RECORD_NAME = b"DATA.TXT"

def xor_dwords(data: bytes) -> int:
    if len(data) % 4:
        raise RuntimeError("xor input must be dword aligned")
    value = 0
    for offset in range(0, len(data), 4):
        value ^= struct.unpack_from("<I", data, offset)[0]
    return value


def set_bit(buf: bytearray, bit: int) -> None:
    buf[bit // 8] |= 1 << (bit & 7)


def put_sector(fh, lba: int, data: bytes) -> None:
    if len(data) != SECTOR:
        raise RuntimeError("sector size mismatch")
    fh.seek(lba * SECTOR)
    fh.write(data)

def build_super() -> bytes:
    sector = bytearray(SECTOR)
    fields = [MAGIC, 1, TOTAL_SECTORS, DIR_LBA, DIR_SECTORS,
              BITMAP_LBA, BITMAP_SECTORS, DATA_LBA,
              MIRROR_LBA - DATA_LBA, 0, 1]
    struct.pack_into("<11I", sector, 0, *fields)
    struct.pack_into("<I", sector, 44, xor_dwords(sector[:44]))
    return bytes(sector)


def build_entry(payload: bytes, start_lba: int, name: bytes) -> tuple[bytes, bytes, int]:
    sectors = (len(payload) + SECTOR - 1) // SECTOR
    if sectors < 1 or sectors > MAX_FILE_SECTORS:
        raise RuntimeError("DBYRT64 HOPFS1 sector count out of range")
    padded = payload + bytes(sectors * SECTOR - len(payload))
    data_xor = xor_dwords(padded)
    first6 = struct.pack("<8s4I", name, start_lba, sectors, len(payload), 1)
    meta_xor = xor_dwords(first6) ^ data_xor
    entry = struct.pack("<8s6I", name, start_lba, sectors,
                        len(payload), 1, meta_xor, data_xor)
    return entry, padded, sectors

def build(source: Path, module: Path, output: Path, empty: bool = False,
          user: Path | None = None, data: Path | None = None) -> None:
    if source.stat().st_size != TOTAL_SECTORS * SECTOR:
        raise RuntimeError("source image must be exactly 16 MiB")
    directory = bytearray(DIR_SECTORS * SECTOR)
    bitmap = bytearray(BITMAP_SECTORS * SECTOR)
    for bit in range(DATA_LBA):
        set_bit(bitmap, bit)
    files: list[tuple[bytes, bytes, int]] = []
    next_lba = DATA_LBA
    module_bytes = b"" if empty else module.read_bytes()
    if not empty:
        entry, padded, sectors = build_entry(module_bytes, next_lba, RECORD_NAME)
        files.append((entry, padded, next_lba))
        next_lba += sectors
    if user is not None:
        user_bytes = user.read_bytes()
        entry, padded, sectors = build_entry(user_bytes, next_lba, USER_RECORD_NAME)
        files.append((entry, padded, next_lba))
        next_lba += sectors
    if data is not None:
        data_bytes = data.read_bytes()
        entry, padded, sectors = build_entry(data_bytes, next_lba, DATA_RECORD_NAME)
        files.append((entry, padded, next_lba))
        next_lba += sectors
    for index, (entry, padded, start_lba) in enumerate(files):
        start = index * len(entry)
        directory[start:start + len(entry)] = entry
        sectors = len(padded) // SECTOR
        for bit in range(start_lba, start_lba + sectors):
            set_bit(bitmap, bit)
    set_bit(bitmap, MIRROR_LBA)
    output.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, output)
    with output.open("r+b") as fh:
        super_sector = build_super()
        put_sector(fh, SUPER_LBA, super_sector)
        put_sector(fh, MIRROR_LBA, super_sector)
        for index in range(DIR_SECTORS):
            start = index * SECTOR
            put_sector(fh, DIR_LBA + index, bytes(directory[start:start + SECTOR]))
        for index in range(BITMAP_SECTORS):
            start = index * SECTOR
            put_sector(fh, BITMAP_LBA + index, bytes(bitmap[start:start + SECTOR]))
        for _, padded, start_lba in files:
            for offset in range(0, len(padded), SECTOR):
                put_sector(fh, start_lba + offset // SECTOR, padded[offset:offset + SECTOR])
    mode = "empty" if empty else "module"
    total_sectors = next_lba - DATA_LBA
    print(f"HOPFS1 DBYRT64 FIXTURE PASS mode={mode} files={len(files)} "
          f"sectors={total_sectors} module_bytes={len(module_bytes)}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("module", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--empty", action="store_true")
    parser.add_argument("--user", type=Path)
    parser.add_argument("--data", type=Path)
    args = parser.parse_args()
    build(args.source, args.module, args.output, args.empty, args.user, args.data)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
