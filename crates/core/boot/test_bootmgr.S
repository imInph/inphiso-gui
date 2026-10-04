/*
 * A stand-in BOOTMGR for testing inphiso's BIOS boot code under QEMU.
 * Copyright (C) 2026 imInph. GPL-3.0-or-later.
 *
 * Loaded at 2000:0000 by vbr.S. Checks that:
 *   - DL holds a hard disk drive number (0x80..),
 *   - the boot sector with its 55AA signature is still at 0000:7C00,
 *   - the whole file arrived: the last 4 bytes (at a known offset) hold a magic,
 * then reports through QEMU's isa-debug-exit device on port 0xF4:
 *   write 0x01 -> QEMU exits with status 3 (pass), 0x02 -> status 5 (fail).
 * It also prints "OK" / "BAD" to COM1 for humans.
 *
 * The QEMU test pads this to FILE_SIZE bytes with MAGIC in the last 4 bytes.
 */
#define FILE_SIZE   0x70000         /* 448 KiB: well over 64 KiB, spans several FAT sectors */
#define MAGIC       0x4f534950      /* "PISO" */

    .code16
    .text
    .globl _start
_start:
    cli
    movw    %cs, %ax
    movw    %ax, %ds
    xorw    %ax, %ax
    movw    %ax, %ss
    movw    $0x7000, %sp

    cmpb    $0x80, %dl
    jb      bad

    xorw    %ax, %ax
    movw    %ax, %es
    cmpw    $0xaa55, %es:0x7dfe
    jne     bad

    /* Last dword of the file: linear 0x20000 + FILE_SIZE - 4. */
    movw    $((0x20000 + FILE_SIZE - 16) >> 4), %ax
    movw    %ax, %es
    cmpl    $MAGIC, %es:12
    jne     bad

    movb    $'O', %al
    call    serial
    movb    $'K', %al
    call    serial
    movb    $0x01, %al
    outb    %al, $0xf4
    jmp     halt

bad:
    movb    $'B', %al
    call    serial
    movb    $0x02, %al
    outb    %al, $0xf4
halt:
    hlt
    jmp     halt

serial:
    movw    $0x3f8, %dx
    outb    %al, %dx
    ret
