#!/usr/bin/env bash
# Assembles the BIOS boot code into the .bin files that are committed and
# embedded with include_bytes!. Needs only clang and python3: the sources use
# label differences for absolute addresses, so there's nothing to link.
#
#   crates/core/boot/build.sh           rebuild the .bin files
#   crates/core/boot/build.sh --check   fail if the committed .bin files are stale
set -euo pipefail
cd "$(dirname "$0")"

out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

extract() { # extract <object> <output>: the .text bytes of a 32-bit ELF object
  python3 - "$1" "$2" <<'EOF'
import struct, sys
data = open(sys.argv[1], "rb").read()
shoff, = struct.unpack_from("<I", data, 0x20)
shentsize, shnum, shstrndx = struct.unpack_from("<HHH", data, 0x2E)
sections = [struct.unpack_from("<IIIIIIIIII", data, shoff + i * shentsize) for i in range(shnum)]
strtab = sections[shstrndx]
name = lambda s: data[strtab[4] + s[0]:].split(b"\0", 1)[0].decode()
for s in sections:
    # SHT_REL / SHT_RELA: an absolute address the assembler couldn't resolve.
    if s[1] in (4, 9) and s[5] > 0:
        sys.exit(f"{sys.argv[1]}: relocations in {name(s)}; use label differences")
text = next(s for s in sections if name(s) == ".text")
open(sys.argv[2], "wb").write(data[text[4]:text[4] + text[5]])
EOF
}

build() { # build <name>
  clang --target=i386-unknown-none-elf -c -o "$out/$1.o" "$1.S"
  extract "$out/$1.o" "$out/$1.bin"
}

build mbr
build vbr
build test_bootmgr

python3 - "$out" <<'EOF'
import sys
out = sys.argv[1]
mbr = open(f"{out}/mbr.bin", "rb").read()
vbr = open(f"{out}/vbr.bin", "rb").read()
boot = open(f"{out}/test_bootmgr.bin", "rb").read()
assert len(mbr) <= 440, f"mbr.bin is {len(mbr)} bytes, the limit is 440"
assert len(vbr) == 0x600, f"vbr.bin is {len(vbr)} bytes, expected 1536"
assert vbr[0:3] == b"\xeb\x58\x90" and vbr[0x1fe:0x200] == b"\x55\xaa"
# The test pads the stand-in BOOTMGR to 0x70000 bytes with a magic at the end.
assert len(boot) < 0x70000 - 4
print(f"mbr {len(mbr)} bytes, vbr {len(vbr)} bytes, test BOOTMGR code {len(boot)} bytes")
EOF

if [[ "${1:-}" == "--check" ]]; then
  for f in mbr vbr test_bootmgr; do
    cmp -s "$out/$f.bin" "$f.bin" || { echo "$f.bin is out of date: run crates/core/boot/build.sh"; exit 1; }
  done
  echo "boot code is up to date"
else
  cp "$out"/{mbr,vbr,test_bootmgr}.bin .
fi
