/*
 * inphiso MBR boot code (first 440 bytes of the drive).
 * Copyright (C) 2026 imInph. GPL-3.0-or-later.
 *
 * Relocates itself to 0x0600, finds the active partition and loads its first
 * sector to 0x7C00 with BIOS LBA reads, then jumps there with DL = boot drive
 * and DS:SI pointing at the partition entry, like the classic DOS/Windows MBR.
 *
 * Assembled with `crates/core/boot/build.sh`. Absolute addresses are written
 * as label differences so no linker is needed.
 */
#define A(x) ((x) - _start + 0x0600)

    .code16
    .text
    .globl _start
_start:
    cli
    xorw    %ax, %ax
    movw    %ax, %ss
    movw    $0x7c00, %sp
    movw    %ax, %ds
    movw    %ax, %es
    sti
    cld
    movw    $0x7c00, %si
    movw    $0x0600, %di
    movw    $256, %cx
    rep movsw
    ljmp    $0, $A(relocated)

relocated:
    movb    %dl, A(drive)

    /* LBA extensions are a must; every PC since the late 90s has them. */
    movb    $0x41, %ah
    movw    $0x55aa, %bx
    int     $0x13
    jc      no_ext
    cmpw    $0xaa55, %bx
    jne     no_ext

    movw    $0x07be, %si            /* partition table at 0x0600 + 446 */
    movw    $4, %cx
find:
    testb   $0x80, (%si)
    jnz     found
    addw    $16, %si
    loop    find
    movw    $A(msg_noact), %si
    jmp     fail

found:
    movl    8(%si), %eax            /* partition start LBA */
    movl    %eax, A(dap_lba)
    pushw   %si
    movw    $A(dap), %si
    movb    $0x42, %ah
    movb    A(drive), %dl
    int     $0x13
    popw    %si
    jc      read_fail
    cmpw    $0xaa55, 0x7dfe
    jne     bad_vbr
    movb    A(drive), %dl
    ljmp    $0, $0x7c00

no_ext:
    movw    $A(msg_noext), %si
    jmp     fail
read_fail:
    movw    $A(msg_read), %si
    jmp     fail
bad_vbr:
    movw    $A(msg_vbr), %si
fail:
    lodsb
    testb   %al, %al
    jz      halt
    movb    $0x0e, %ah
    movw    $7, %bx
    int     $0x10
    jmp     fail
halt:
    hlt
    jmp     halt

drive:      .byte 0x80
dap:        .byte 16, 0
            .word 1                 /* one sector */
            .word 0x7c00, 0         /* to 0000:7C00 */
dap_lba:    .long 0, 0
msg_noact:  .asciz "inphiso: no active partition"
msg_noext:  .asciz "inphiso: BIOS lacks LBA support"
msg_read:   .asciz "inphiso: disk read error"
msg_vbr:    .asciz "inphiso: partition not bootable"
