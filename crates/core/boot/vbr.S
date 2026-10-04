/*
 * inphiso FAT32 boot code: loads BOOTMGR for legacy BIOS boot of Windows Setup.
 * Copyright (C) 2026 imInph. GPL-3.0-or-later.
 *
 * Layout of the assembled file (see build.sh):
 *   bytes 0x000-0x002   jump over the BPB
 *   bytes 0x003-0x059   BPB placeholder (the formatter's BPB is kept, never overwritten)
 *   bytes 0x05a-0x1fd   stage 1: sets up, loads stage 2 from reserved sectors 2-3
 *   bytes 0x1fe-0x1ff   0x55 0xAA
 *   bytes 0x200-0x5ff   stage 2: finds BOOTMGR in the root directory, loads it to
 *                       2000:0000 following its FAT cluster chain, jumps to it
 *
 * Like Microsoft's NT6 FAT32 boot sector, BOOTMGR is entered at 2000:0000 with
 * DL = boot drive and this boot sector (with its BPB) still at 0000:7C00.
 *
 * Memory: stack below 0x7C00, stage 2 at 0x7E00, FAT sector buffer at 0x8000,
 * directory sector buffer at 0x9000 (sectors up to 4 KiB), BOOTMGR from 0x20000.
 */
#define A(x) ((x) - _start + 0x7c00)

#define BPB_BYTES   0x7c0b      /* u16 bytes per sector */
#define BPB_SPC     0x7c0d      /* u8  sectors per cluster */
#define BPB_RSVD    0x7c0e      /* u16 reserved sectors */
#define BPB_NFATS   0x7c10      /* u8  number of FATs */
#define BPB_HIDDEN  0x7c1c      /* u32 partition start LBA */
#define BPB_FATSZ   0x7c24      /* u32 sectors per FAT */
#define BPB_ROOT    0x7c2c      /* u32 root directory cluster */
#define BPB_DRIVE   0x7c40      /* u8  BIOS drive number */

#define STAGE2_SECTOR   2
#define STAGE2_COUNT    2
#define LOAD_SEG        0x2000
#define LOAD_LIMIT_SEG  0x9f00  /* BIOS data (EBDA, video) starts around 0x9FC00 */
#define END_OF_CHAIN    0x0ffffff8

    .code16
    .text
    .globl _start
_start:
    .byte   0xeb, 0x58, 0x90        /* jmp short start; nop */
    . = _start + 0x5a

start:
    cli
    xorw    %ax, %ax
    movw    %ax, %ss
    movw    $0x7c00, %sp
    movw    %ax, %ds
    movw    %ax, %es
    sti
    cld
    movb    %dl, BPB_DRIVE

    movl    $STAGE2_SECTOR, %eax
    movw    $0x07e0, %bx
    movw    %bx, %es
    movw    $STAGE2_COUNT, %cx
    call    read
    jmp     stage2

/*
 * read: EAX = partition-relative LBA, ES = destination segment (offset 0),
 * CX = sector count. Advances ES past the data read. Clobbers EAX, CX, DX, SI.
 */
read:
    pushl   %eax
    addl    BPB_HIDDEN, %eax
    movl    %eax, A(dap_lba)
    movw    %es, A(dap_seg)
    movw    $A(dap), %si
    movb    $0x42, %ah
    movb    BPB_DRIVE, %dl
    int     $0x13
    jc      disk_error
    popl    %eax
    incl    %eax
    movw    BPB_BYTES, %dx
    shrw    $4, %dx
    movw    %es, %si
    addw    %dx, %si
    movw    %si, %es
    loop    read
    ret

disk_error:
    movw    $A(msg_disk), %si
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

dap:        .byte 16, 0
            .word 1
            .word 0
dap_seg:    .word 0
dap_lba:    .long 0, 0
msg_disk:   .asciz "inphiso: disk read error"

    . = _start + 0x1fe
    .byte   0x55, 0xaa

/* ---------------- stage 2: reserved sectors 2-3, at 0x7E00 ---------------- */
stage2:
    /* First data sector = reserved + FATs * sectors per FAT. */
    movzbl  BPB_NFATS, %eax
    mull    BPB_FATSZ
    movzwl  BPB_RSVD, %ecx
    addl    %ecx, %eax
    movl    %eax, A(data_start)

    movl    BPB_ROOT, %eax
dir_cluster:
    movl    %eax, A(cur_cluster)
    call    cluster_lba
    movzbw  BPB_SPC, %cx
dir_sector:
    pushw   %cx
    pushl   %eax
    movw    $0x0900, %bx
    movw    %bx, %es
    movw    $1, %cx
    call    read
    xorw    %bx, %bx
    movw    %bx, %es
    movw    $0x9000, %di
    movw    BPB_BYTES, %cx
    shrw    $5, %cx                 /* 32-byte entries per sector */
entry:
    cmpb    $0, (%di)               /* end of directory */
    je      not_found
    testb   $0x18, 11(%di)          /* skip directories, volume labels and LFN entries */
    jnz     next_entry
    pushw   %cx
    pushw   %di
    movw    $A(name), %si
    movw    $11, %cx
    repe cmpsb
    popw    %di
    popw    %cx
    je      found
next_entry:
    addw    $32, %di
    loop    entry
    popl    %eax
    incl    %eax
    popw    %cx
    loop    dir_sector
    movl    A(cur_cluster), %eax
    call    next_cluster
    cmpl    $END_OF_CHAIN, %eax
    jb      dir_cluster
not_found:
    movw    $A(msg_nofile), %si
    jmp     fail

found:
    addw    $6, %sp                 /* drop the saved sector LBA and count */
    movw    20(%di), %ax            /* first cluster, high word */
    shll    $16, %eax
    movw    26(%di), %ax            /* low word */
    movw    $LOAD_SEG, %bx
    movw    %bx, %es
load_cluster:
    movl    %eax, A(cur_cluster)
    /* Refuse before reading a cluster that would run into BIOS memory. */
    movzbl  BPB_SPC, %eax
    movzwl  BPB_BYTES, %ecx
    mull    %ecx
    shrl    $4, %eax                /* paragraphs per cluster */
    movw    %es, %cx
    movzwl  %cx, %ecx
    addl    %ecx, %eax
    cmpl    $LOAD_LIMIT_SEG, %eax
    ja      too_big
    movl    A(cur_cluster), %eax
    call    cluster_lba
    movzbw  BPB_SPC, %cx
    call    read
    movl    A(cur_cluster), %eax
    call    next_cluster
    cmpl    $END_OF_CHAIN, %eax
    jb      load_cluster

    movb    BPB_DRIVE, %dl
    ljmp    $LOAD_SEG, $0

too_big:
    movw    $A(msg_big), %si
    jmp     fail

/* cluster_lba: EAX = cluster -> EAX = partition-relative LBA of its first sector. */
cluster_lba:
    subl    $2, %eax
    movzbl  BPB_SPC, %ecx
    mull    %ecx
    addl    A(data_start), %eax
    ret

/* next_cluster: EAX = cluster -> EAX = next cluster from the FAT. Preserves ES. */
next_cluster:
    pushw   %es
    shll    $2, %eax                /* byte offset in the FAT */
    movzwl  BPB_BYTES, %ecx
    xorl    %edx, %edx
    divl    %ecx                    /* EAX = FAT sector, EDX = offset in it */
    movzwl  BPB_RSVD, %ecx
    addl    %ecx, %eax
    pushw   %dx
    cmpl    A(fat_cached), %eax
    je      1f
    movl    %eax, A(fat_cached)
    movw    $0x0800, %bx
    movw    %bx, %es
    movw    $1, %cx
    call    read
1:
    popw    %si
    movl    0x8000(%si), %eax
    andl    $0x0fffffff, %eax
    popw    %es
    ret

data_start:  .long 0
cur_cluster: .long 0
fat_cached:  .long 0xffffffff
name:        .ascii "BOOTMGR    "
msg_nofile:  .asciz "inphiso: BOOTMGR is missing"
msg_big:     .asciz "inphiso: BOOTMGR is too big"

    . = _start + 0x600
