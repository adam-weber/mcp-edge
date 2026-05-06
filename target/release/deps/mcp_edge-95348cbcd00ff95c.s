	.build_version macos, 11, 0
	.section	__TEXT,__text,regular,pure_instructions
	.globl	__ZN8mcp_edge11extract_obj17h1f1c3050dfdadd6eE
	.p2align	2
__ZN8mcp_edge11extract_obj17h1f1c3050dfdadd6eE:
	.cfi_startproc
	stp	x26, x25, [sp, #-80]!
	.cfi_def_cfa_offset 80
	stp	x24, x23, [sp, #16]
	stp	x22, x21, [sp, #32]
	stp	x20, x19, [sp, #48]
	stp	x29, x30, [sp, #64]
	add	x29, sp, #64
	.cfi_def_cfa w29, 16
	.cfi_offset w30, -8
	.cfi_offset w29, -16
	.cfi_offset w19, -24
	.cfi_offset w20, -32
	.cfi_offset w21, -40
	.cfi_offset w22, -48
	.cfi_offset w23, -56
	.cfi_offset w24, -64
	.cfi_offset w25, -72
	.cfi_offset w26, -80
	.cfi_remember_state
	cbz	x3, LBB0_28
	mov	x19, x3
	mov	x20, x1
	subs	x25, x1, x3
	b.lo	LBB0_10
	mov	x22, x2
	mov	x21, x0
	mov	x23, #0
	mov	x24, x25
	mov	x26, x20
LBB0_3:
	add	x0, x21, x23
	mov	x1, x22
	mov	x2, x19
	bl	_memcmp
	cbz	w0, LBB0_5
	sub	x26, x26, #1
	add	x23, x23, #1
	sub	x24, x24, #1
	cmp	x19, x26
	b.ls	LBB0_3
	b	LBB0_10
LBB0_5:
	add	x0, x19, x23
	cmp	x0, x20
	b.hi	LBB0_29
	subs	x10, x25, x23
	b.eq	LBB0_10
	mov	x8, #0
	add	x9, x21, x19
	mov	w11, #1
	mov	x12, #9728
	movk	x12, #1, lsl #32
LBB0_8:
	add	x13, x9, x8
	ldrb	w14, [x13, x23]
	cmp	w14, #32
	lsl	x14, x11, x14
	and	x14, x14, x12
	ccmp	x14, #0, #4, ls
	b.eq	LBB0_12
	add	x8, x8, #1
	cmp	x10, x8
	b.ne	LBB0_8
LBB0_10:
	mov	x0, #0
LBB0_11:
	.cfi_def_cfa wsp, 80
	ldp	x29, x30, [sp, #64]
	ldp	x20, x19, [sp, #48]
	ldp	x22, x21, [sp, #32]
	ldp	x24, x23, [sp, #16]
	ldp	x26, x25, [sp], #80
	.cfi_def_cfa_offset 0
	.cfi_restore w30
	.cfi_restore w29
	.cfi_restore w19
	.cfi_restore w20
	.cfi_restore w21
	.cfi_restore w22
	.cfi_restore w23
	.cfi_restore w24
	.cfi_restore w25
	.cfi_restore w26
	ret
LBB0_12:
	.cfi_restore_state
	ldrb	w10, [x13, x23]
	cmp	w10, #123
	b.ne	LBB0_10
	mov	x13, #0
	mov	x11, #0
	sub	x12, x24, x8
	add	x10, x9, x23
	add	x10, x10, x8
	b	LBB0_16
LBB0_14:
	add	x11, x11, #1
LBB0_15:
	mov	x0, #0
	add	x13, x13, #1
	cmp	x13, x12
	b.hs	LBB0_11
LBB0_16:
	add	x14, x8, x13
	add	x14, x9, x14
	ldrb	w14, [x14, x23]
	cmp	w14, #125
	b.eq	LBB0_26
	cmp	w14, #123
	b.eq	LBB0_14
	cmp	w14, #34
	b.ne	LBB0_15
	add	x14, x13, #1
	b	LBB0_22
LBB0_20:
	add	x14, x13, #2
LBB0_21:
	mov	x13, x14
	add	x14, x14, #1
LBB0_22:
	cmp	x14, x12
	b.hs	LBB0_25
	add	x15, x8, x14
	add	x15, x9, x15
	ldrb	w15, [x15, x23]
	cmp	w15, #92
	b.eq	LBB0_20
	cmp	w15, #34
	b.ne	LBB0_21
LBB0_25:
	mov	x13, x14
	b	LBB0_15
LBB0_26:
	subs	x11, x11, #1
	b.ne	LBB0_15
	add	x1, x13, #1
	mov	x0, x10
	b	LBB0_11
LBB0_28:
Lloh0:
	adrp	x0, l_anon.a9251df030306a5b454b8fa8315727c3.10@PAGE
Lloh1:
	add	x0, x0, l_anon.a9251df030306a5b454b8fa8315727c3.10@PAGEOFF
Lloh2:
	adrp	x2, l_anon.a9251df030306a5b454b8fa8315727c3.11@PAGE
Lloh3:
	add	x2, x2, l_anon.a9251df030306a5b454b8fa8315727c3.11@PAGEOFF
	mov	w1, #28
	bl	__ZN4core6option13expect_failed17h0ccd2b94a57b79d0E
LBB0_29:
Lloh4:
	adrp	x3, l_anon.a9251df030306a5b454b8fa8315727c3.1@PAGE
Lloh5:
	add	x3, x3, l_anon.a9251df030306a5b454b8fa8315727c3.1@PAGEOFF
	mov	x1, x20
	mov	x2, x20
	bl	__ZN4core5slice5index16slice_index_fail17hefbaa1bb9611e99cE
	.loh AdrpAdd	Lloh2, Lloh3
	.loh AdrpAdd	Lloh0, Lloh1
	.loh AdrpAdd	Lloh4, Lloh5
	.cfi_endproc

	.globl	__ZN8mcp_edge11extract_str17hf7d863f5a61249f1E
	.p2align	2
__ZN8mcp_edge11extract_str17hf7d863f5a61249f1E:
	.cfi_startproc
	stp	x26, x25, [sp, #-80]!
	.cfi_def_cfa_offset 80
	stp	x24, x23, [sp, #16]
	stp	x22, x21, [sp, #32]
	stp	x20, x19, [sp, #48]
	stp	x29, x30, [sp, #64]
	add	x29, sp, #64
	.cfi_def_cfa w29, 16
	.cfi_offset w30, -8
	.cfi_offset w29, -16
	.cfi_offset w19, -24
	.cfi_offset w20, -32
	.cfi_offset w21, -40
	.cfi_offset w22, -48
	.cfi_offset w23, -56
	.cfi_offset w24, -64
	.cfi_offset w25, -72
	.cfi_offset w26, -80
	.cfi_remember_state
	cbz	x3, LBB1_20
	mov	x19, x3
	mov	x20, x1
	subs	x25, x1, x3
	b.lo	LBB1_10
	mov	x22, x2
	mov	x21, x0
	mov	x23, #0
	mvn	x8, x19
	add	x24, x8, x20
	mov	x26, x20
LBB1_3:
	add	x0, x21, x23
	mov	x1, x22
	mov	x2, x19
	bl	_memcmp
	cbz	w0, LBB1_5
	sub	x26, x26, #1
	add	x23, x23, #1
	sub	x24, x24, #1
	cmp	x19, x26
	b.ls	LBB1_3
	b	LBB1_10
LBB1_5:
	add	x0, x19, x23
	cmp	x0, x20
	b.hi	LBB1_21
	subs	x10, x25, x23
	b.eq	LBB1_10
	mov	x8, #0
	add	x9, x21, x19
	mov	w11, #1
	mov	x12, #9728
	movk	x12, #1, lsl #32
LBB1_8:
	add	x13, x9, x8
	ldrb	w14, [x13, x23]
	cmp	w14, #32
	lsl	x14, x11, x14
	and	x14, x14, x12
	ccmp	x14, #0, #4, ls
	b.eq	LBB1_12
	add	x8, x8, #1
	cmp	x10, x8
	b.ne	LBB1_8
LBB1_10:
	mov	x0, #0
LBB1_11:
	.cfi_def_cfa wsp, 80
	ldp	x29, x30, [sp, #64]
	ldp	x20, x19, [sp, #48]
	ldp	x22, x21, [sp, #32]
	ldp	x24, x23, [sp, #16]
	ldp	x26, x25, [sp], #80
	.cfi_def_cfa_offset 0
	.cfi_restore w30
	.cfi_restore w29
	.cfi_restore w19
	.cfi_restore w20
	.cfi_restore w21
	.cfi_restore w22
	.cfi_restore w23
	.cfi_restore w24
	.cfi_restore w25
	.cfi_restore w26
	ret
LBB1_12:
	.cfi_restore_state
	ldrb	w10, [x13, x23]
	cmp	w10, #34
	b.ne	LBB1_10
	mvn	x10, x23
	add	x10, x10, x25
	cmp	x10, x8
	b.eq	LBB1_10
	mov	x12, #0
	sub	x10, x24, x8
	add	x11, x9, x23
	add	x11, x11, x8
	add	x11, x11, #1
	b	LBB1_16
LBB1_15:
	mov	w13, #2
	mov	x0, #0
	mov	x1, x12
	add	x12, x12, x13
	cmp	x12, x10
	b.hs	LBB1_11
LBB1_16:
	add	x13, x8, x12
	add	x14, x9, x23
	add	x13, x14, x13
	ldrb	w13, [x13, #1]
	cmp	w13, #92
	b.eq	LBB1_15
	cmp	w13, #34
	b.eq	LBB1_19
	mov	w13, #1
	mov	x0, #0
	mov	x1, x12
	add	x12, x12, x13
	cmp	x12, x10
	b.lo	LBB1_16
	b	LBB1_11
LBB1_19:
	mov	x1, x12
	mov	x0, x11
	b	LBB1_11
LBB1_20:
Lloh6:
	adrp	x0, l_anon.a9251df030306a5b454b8fa8315727c3.10@PAGE
Lloh7:
	add	x0, x0, l_anon.a9251df030306a5b454b8fa8315727c3.10@PAGEOFF
Lloh8:
	adrp	x2, l_anon.a9251df030306a5b454b8fa8315727c3.11@PAGE
Lloh9:
	add	x2, x2, l_anon.a9251df030306a5b454b8fa8315727c3.11@PAGEOFF
	mov	w1, #28
	bl	__ZN4core6option13expect_failed17h0ccd2b94a57b79d0E
LBB1_21:
Lloh10:
	adrp	x3, l_anon.a9251df030306a5b454b8fa8315727c3.2@PAGE
Lloh11:
	add	x3, x3, l_anon.a9251df030306a5b454b8fa8315727c3.2@PAGEOFF
	mov	x1, x20
	mov	x2, x20
	bl	__ZN4core5slice5index16slice_index_fail17hefbaa1bb9611e99cE
	.loh AdrpAdd	Lloh8, Lloh9
	.loh AdrpAdd	Lloh6, Lloh7
	.loh AdrpAdd	Lloh10, Lloh11
	.cfi_endproc

	.globl	__ZN8mcp_edge11extract_u6417hd5a0cb2f8ee060d7E
	.p2align	2
__ZN8mcp_edge11extract_u6417hd5a0cb2f8ee060d7E:
	.cfi_startproc
	stp	x28, x27, [sp, #-96]!
	.cfi_def_cfa_offset 96
	stp	x26, x25, [sp, #16]
	stp	x24, x23, [sp, #32]
	stp	x22, x21, [sp, #48]
	stp	x20, x19, [sp, #64]
	stp	x29, x30, [sp, #80]
	add	x29, sp, #80
	.cfi_def_cfa w29, 16
	.cfi_offset w30, -8
	.cfi_offset w29, -16
	.cfi_offset w19, -24
	.cfi_offset w20, -32
	.cfi_offset w21, -40
	.cfi_offset w22, -48
	.cfi_offset w23, -56
	.cfi_offset w24, -64
	.cfi_offset w25, -72
	.cfi_offset w26, -80
	.cfi_offset w27, -88
	.cfi_offset w28, -96
	.cfi_remember_state
	cbz	x3, LBB2_22
	mov	x20, x3
	mov	x21, x1
	subs	x24, x1, x3
	b.lo	LBB2_20
	mov	x23, x2
	mov	x22, x0
	mov	x26, #0
	add	x25, x0, x20
	mov	x27, x21
	mov	x19, x0
LBB2_3:
	mov	x0, x19
	mov	x1, x23
	mov	x2, x20
	bl	_memcmp
	cbz	w0, LBB2_5
	add	x19, x19, #1
	sub	x27, x27, #1
	add	x26, x26, #1
	sub	x24, x24, #1
	add	x25, x25, #1
	cmp	x20, x27
	b.ls	LBB2_3
	b	LBB2_20
LBB2_5:
	add	x0, x26, x20
	subs	x8, x21, x0
	b.lo	LBB2_24
	add	x10, x22, x0
	add	x11, x22, x21
	mov	x9, #0
	b.eq	LBB2_11
	mov	w12, #1
	mov	x13, #9728
	movk	x13, #1, lsl #32
LBB2_8:
	ldrb	w14, [x25, x9]
	cmp	w14, #32
	lsl	x14, x12, x14
	and	x14, x14, x13
	ccmp	x14, #0, #4, ls
	b.eq	LBB2_11
	add	x9, x9, #1
	cmp	x24, x9
	b.ne	LBB2_8
	mov	x9, x8
LBB2_11:
	sub	x2, x8, x9
	add	x8, x10, x9
	cmp	x8, x11
	mov	x1, x2
	b.eq	LBB2_16
	mov	x1, #0
	sub	x8, x24, x9
	add	x10, x25, x9
LBB2_13:
	ldrb	w11, [x10, x1]
	sub	w11, w11, #58
	cmn	w11, #10
	b.lo	LBB2_16
	add	x1, x1, #1
	cmp	x8, x1
	b.ne	LBB2_13
	mov	x1, x2
LBB2_16:
	cbz	x1, LBB2_20
	cmp	x1, x2
	b.hi	LBB2_23
	mov	x8, #0
	add	x9, x9, x20
	mov	w10, #10
	mov	x11, #-1
	mov	w0, #1
LBB2_19:
	ldrb	w12, [x19, x9]
	umulh	x13, x8, x10
	add	x8, x8, x8, lsl #2
	lsl	x8, x8, #1
	cmp	x13, #0
	csel	x8, x8, x11, eq
	sub	w12, w12, #48
	adds	x8, x8, w12, uxtb
	csinv	x8, x8, xzr, lo
	add	x9, x9, #1
	subs	x1, x1, #1
	b.ne	LBB2_19
	b	LBB2_21
LBB2_20:
	mov	x0, #0
LBB2_21:
	mov	x1, x8
	.cfi_def_cfa wsp, 96
	ldp	x29, x30, [sp, #80]
	ldp	x20, x19, [sp, #64]
	ldp	x22, x21, [sp, #48]
	ldp	x24, x23, [sp, #32]
	ldp	x26, x25, [sp, #16]
	ldp	x28, x27, [sp], #96
	.cfi_def_cfa_offset 0
	.cfi_restore w30
	.cfi_restore w29
	.cfi_restore w19
	.cfi_restore w20
	.cfi_restore w21
	.cfi_restore w22
	.cfi_restore w23
	.cfi_restore w24
	.cfi_restore w25
	.cfi_restore w26
	.cfi_restore w27
	.cfi_restore w28
	ret
LBB2_22:
	.cfi_restore_state
Lloh12:
	adrp	x0, l_anon.a9251df030306a5b454b8fa8315727c3.10@PAGE
Lloh13:
	add	x0, x0, l_anon.a9251df030306a5b454b8fa8315727c3.10@PAGEOFF
Lloh14:
	adrp	x2, l_anon.a9251df030306a5b454b8fa8315727c3.11@PAGE
Lloh15:
	add	x2, x2, l_anon.a9251df030306a5b454b8fa8315727c3.11@PAGEOFF
	mov	w1, #28
	bl	__ZN4core6option13expect_failed17h0ccd2b94a57b79d0E
LBB2_23:
Lloh16:
	adrp	x3, l_anon.a9251df030306a5b454b8fa8315727c3.3@PAGE
Lloh17:
	add	x3, x3, l_anon.a9251df030306a5b454b8fa8315727c3.3@PAGEOFF
	mov	x0, #0
	bl	__ZN4core5slice5index16slice_index_fail17hefbaa1bb9611e99cE
LBB2_24:
Lloh18:
	adrp	x3, l_anon.a9251df030306a5b454b8fa8315727c3.4@PAGE
Lloh19:
	add	x3, x3, l_anon.a9251df030306a5b454b8fa8315727c3.4@PAGEOFF
	mov	x1, x21
	mov	x2, x21
	bl	__ZN4core5slice5index16slice_index_fail17hefbaa1bb9611e99cE
	.loh AdrpAdd	Lloh14, Lloh15
	.loh AdrpAdd	Lloh12, Lloh13
	.loh AdrpAdd	Lloh16, Lloh17
	.loh AdrpAdd	Lloh18, Lloh19
	.cfi_endproc

	.globl	__ZN8mcp_edge6Writer1s17h071749ee80e43f15E
	.p2align	2
__ZN8mcp_edge6Writer1s17h071749ee80e43f15E:
	.cfi_startproc
	stp	x20, x19, [sp, #-32]!
	.cfi_def_cfa_offset 32
	stp	x29, x30, [sp, #16]
	add	x29, sp, #16
	.cfi_def_cfa w29, 16
	.cfi_offset w30, -8
	.cfi_offset w29, -16
	.cfi_offset w19, -24
	.cfi_offset w20, -32
	.cfi_remember_state
	mov	x9, x2
	mov	x20, x0
	ldp	x2, x0, [x0, #8]
	subs	x10, x2, x0
	csel	x10, xzr, x10, lo
	cmp	x10, x9
	csel	x9, x10, x9, lo
	adds	x19, x9, x0
	b.hs	LBB3_3
	cmp	x19, x2
	b.hi	LBB3_3
	ldr	x10, [x20]
	add	x0, x10, x0
	mov	x2, x9
	bl	_memcpy
	str	x19, [x20, #16]
	mov	x0, x20
	.cfi_def_cfa wsp, 32
	ldp	x29, x30, [sp, #16]
	ldp	x20, x19, [sp], #32
	.cfi_def_cfa_offset 0
	.cfi_restore w30
	.cfi_restore w29
	.cfi_restore w19
	.cfi_restore w20
	ret
LBB3_3:
	.cfi_restore_state
Lloh20:
	adrp	x3, l_anon.a9251df030306a5b454b8fa8315727c3.9@PAGE
Lloh21:
	add	x3, x3, l_anon.a9251df030306a5b454b8fa8315727c3.9@PAGEOFF
	mov	x1, x19
	bl	__ZN4core5slice5index16slice_index_fail17hefbaa1bb9611e99cE
	.loh AdrpAdd	Lloh20, Lloh21
	.cfi_endproc

	.globl	__ZN8mcp_edge6Writer1u17h01d45ee9b9ccbe60E
	.p2align	2
__ZN8mcp_edge6Writer1u17h01d45ee9b9ccbe60E:
	.cfi_startproc
	sub	sp, sp, #64
	.cfi_def_cfa_offset 64
	stp	x20, x19, [sp, #32]
	stp	x29, x30, [sp, #48]
	add	x29, sp, #48
	.cfi_def_cfa w29, 16
	.cfi_offset w30, -8
	.cfi_offset w29, -16
	.cfi_offset w19, -24
	.cfi_offset w20, -32
	.cfi_remember_state
	stp	xzr, xzr, [sp, #8]
	str	wzr, [sp, #24]
	add	x9, sp, #8
	add	x8, x9, #19
	cbz	x1, LBB4_3
	mov	x10, #-3689348814741910324
	movk	x10, #52429
	umulh	x10, x1, x10
	lsr	x10, x10, #3
	mov	w11, #10
	msub	w12, w10, w11, w1
	orr	w12, w12, #0x30
	strb	w12, [sp, #27]
	cmp	x1, #10
	b.hs	LBB4_4
	mov	w10, #1
	b	LBB4_41
LBB4_3:
	mov	w9, #48
	strb	w9, [sp, #27]
	mov	w10, #1
	b	LBB4_41
LBB4_4:
	mov	x8, #-7378697629483820647
	eor	x8, x8, #0x8000000000000003
	umulh	x12, x10, x8
	msub	w10, w12, w11, w10
	orr	w10, w10, #0x30
	strb	w10, [sp, #26]
	cmp	x1, #100
	b.hs	LBB4_6
	add	x8, x9, #18
	mov	w10, #2
	b	LBB4_41
LBB4_6:
	lsr	x10, x1, #2
	mov	x11, #62915
	movk	x11, #23592, lsl #16
	movk	x11, #49807, lsl #32
	movk	x11, #10485, lsl #48
	umulh	x10, x10, x11
	lsr	x10, x10, #2
	umulh	x11, x10, x8
	mov	w8, #10
	msub	w10, w11, w8, w10
	orr	w10, w10, #0x30
	strb	w10, [sp, #25]
	cmp	x1, #1000
	b.hs	LBB4_8
	add	x8, x9, #17
	mov	w10, #3
	b	LBB4_41
LBB4_8:
	lsr	x10, x1, #3
	mov	x11, #63439
	movk	x11, #58195, lsl #16
	movk	x11, #39845, lsl #32
	movk	x11, #8388, lsl #48
	umulh	x10, x10, x11
	lsr	x11, x10, #4
	mov	x10, #-7378697629483820647
	eor	x10, x10, #0x8000000000000003
	umulh	x12, x11, x10
	msub	w8, w12, w8, w11
	orr	w8, w8, #0x30
	strb	w8, [sp, #24]
	lsr	x8, x1, #4
	cmp	x8, #625
	b.hs	LBB4_10
	add	x8, x9, #16
	mov	w10, #4
	b	LBB4_41
LBB4_10:
	mov	x8, #22859
	movk	x8, #14470, lsl #16
	movk	x8, #50646, lsl #32
	movk	x8, #13421, lsl #48
	umulh	x8, x1, x8
	lsr	x11, x8, #11
	umulh	x10, x11, x10
	mov	w8, #10
	msub	w10, w10, w8, w11
	orr	w10, w10, #0x30
	strb	w10, [sp, #23]
	lsr	x10, x1, #5
	cmp	x10, #3125
	b.hs	LBB4_12
	add	x8, x9, #15
	mov	w10, #5
	b	LBB4_41
LBB4_12:
	mov	x11, #30787
	movk	x11, #29108, lsl #16
	movk	x11, #23236, lsl #32
	movk	x11, #2684, lsl #48
	umulh	x10, x10, x11
	lsr	x11, x10, #7
	mov	x10, #-7378697629483820647
	eor	x10, x10, #0x8000000000000003
	umulh	x12, x11, x10
	msub	w8, w12, w8, w11
	orr	w8, w8, #0x30
	strb	w8, [sp, #22]
	mov	w8, #16960
	movk	w8, #15, lsl #16
	cmp	x1, x8
	b.hs	LBB4_14
	add	x8, x9, #14
	mov	w10, #6
	b	LBB4_41
LBB4_14:
	mov	x8, #13531
	movk	x8, #55222, lsl #16
	movk	x8, #56962, lsl #32
	movk	x8, #17179, lsl #48
	umulh	x8, x1, x8
	lsr	x11, x8, #18
	umulh	x10, x11, x10
	mov	w8, #10
	msub	w10, w10, w8, w11
	orr	w10, w10, #0x30
	strb	w10, [sp, #21]
	mov	w10, #38528
	movk	w10, #152, lsl #16
	cmp	x1, x10
	b.hs	LBB4_16
	add	x8, x9, #13
	mov	w10, #7
	b	LBB4_41
LBB4_16:
	mov	x10, #17085
	movk	x10, #58746, lsl #16
	movk	x10, #38101, lsl #32
	movk	x10, #54975, lsl #48
	umulh	x10, x1, x10
	lsr	x11, x10, #23
	mov	x10, #-7378697629483820647
	eor	x10, x10, #0x8000000000000003
	umulh	x12, x11, x10
	msub	w8, w12, w8, w11
	orr	w8, w8, #0x30
	strb	w8, [sp, #20]
	mov	w8, #57600
	movk	w8, #1525, lsl #16
	cmp	x1, x8
	b.hs	LBB4_18
	add	x8, x9, #12
	mov	w10, #8
	b	LBB4_41
LBB4_18:
	mov	x8, #52989
	movk	x8, #33889, lsl #16
	movk	x8, #30481, lsl #32
	movk	x8, #43980, lsl #48
	umulh	x8, x1, x8
	lsr	x8, x8, #26
	umulh	x11, x8, x10
	mov	w10, #10
	msub	w8, w11, w10, w8
	orr	w8, w8, #0x30
	strb	w8, [sp, #19]
	mov	w8, #51712
	movk	w8, #15258, lsl #16
	cmp	x1, x8
	b.hs	LBB4_20
	add	x8, x9, #11
	mov	w10, #9
	b	LBB4_41
LBB4_20:
	lsr	x8, x1, #9
	mov	x11, #23123
	movk	x11, #41115, lsl #16
	movk	x11, #47151, lsl #32
	movk	x11, #68, lsl #48
	umulh	x8, x8, x11
	lsr	x8, x8, #11
	mov	x11, #-7378697629483820647
	eor	x11, x11, #0x8000000000000003
	umulh	x11, x8, x11
	msub	w8, w11, w10, w8
	orr	w8, w8, #0x30
	strb	w8, [sp, #18]
	mov	x8, #58368
	movk	x8, #21515, lsl #16
	movk	x8, #2, lsl #32
	cmp	x1, x8
	b.hs	LBB4_22
	add	x8, x9, #10
	b	LBB4_41
LBB4_22:
	mov	x8, #54719
	movk	x8, #48621, lsl #16
	movk	x8, #65230, lsl #32
	movk	x8, #56294, lsl #48
	umulh	x8, x1, x8
	lsr	x10, x8, #33
	mov	w8, #26215
	movk	w8, #26214, lsl #16
	umull	x8, w10, w8
	lsr	x11, x8, #34
	mov	w8, #10
	msub	w10, w11, w8, w10
	orr	w10, w10, #0x30
	strb	w10, [sp, #17]
	mov	x10, #59392
	movk	x10, #18550, lsl #16
	movk	x10, #23, lsl #32
	cmp	x1, x10
	b.hs	LBB4_24
	add	x8, x9, #9
	mov	w10, #11
	b	LBB4_41
LBB4_24:
	mov	x10, #43775
	movk	x10, #52004, lsl #16
	movk	x10, #65291, lsl #32
	movk	x10, #45035, lsl #48
	umulh	x10, x1, x10
	lsr	x10, x10, #36
	mov	w11, #39322
	movk	w11, #6553, lsl #16
	umull	x11, w10, w11
	lsr	x11, x11, #32
	msub	w8, w11, w8, w10
	orr	w8, w8, #0x30
	strb	w8, [sp, #16]
	mov	x8, #4096
	movk	x8, #54437, lsl #16
	movk	x8, #232, lsl #32
	cmp	x1, x8
	b.hs	LBB4_26
	add	x8, x9, #8
	mov	w10, #12
	b	LBB4_41
LBB4_26:
	mov	x8, #8755
	movk	x8, #23508, lsl #16
	movk	x8, #13058, lsl #32
	movk	x8, #9007, lsl #48
	umulh	x8, x1, x8
	lsr	x10, x8, #37
	mov	w8, #39322
	movk	w8, #6553, lsl #16
	umull	x8, w10, w8
	lsr	x11, x8, #32
	mov	w8, #10
	msub	w10, w11, w8, w10
	orr	w10, w10, #0x30
	strb	w10, [sp, #15]
	mov	x10, #40960
	movk	x10, #20082, lsl #16
	movk	x10, #2328, lsl #32
	cmp	x1, x10
	b.hs	LBB4_28
	orr	x8, x9, #0x7
	mov	w10, #13
	b	LBB4_41
LBB4_28:
	mov	x10, #901
	movk	x10, #37613, lsl #16
	movk	x10, #34000, lsl #32
	movk	x10, #14411, lsl #48
	umulh	x10, x1, x10
	lsr	x10, x10, #41
	mov	w11, #39322
	movk	w11, #6553, lsl #16
	umull	x11, w10, w11
	lsr	x11, x11, #32
	msub	w8, w11, w8, w10
	orr	w8, w8, #0x30
	strb	w8, [sp, #14]
	mov	x8, #16384
	movk	x8, #4218, lsl #16
	movk	x8, #23283, lsl #32
	cmp	x1, x8
	b.hs	LBB4_30
	orr	x8, x9, #0x6
	mov	w10, #14
	b	LBB4_41
LBB4_30:
	mov	x8, #52609
	movk	x8, #20629, lsl #16
	movk	x8, #19907, lsl #32
	movk	x8, #2882, lsl #48
	umulh	x8, x1, x8
	lsr	x10, x8, #42
	mov	w8, #39322
	movk	w8, #6553, lsl #16
	umull	x8, w10, w8
	lsr	x11, x8, #32
	mov	w8, #10
	msub	w10, w11, w8, w10
	orr	w10, w10, #0x30
	strb	w10, [sp, #13]
	mov	x10, #1125899906809856
	movk	x10, #42182, lsl #16
	movk	x10, #36222, lsl #32
	cmp	x1, x10
	b.hs	LBB4_32
	mov	w8, #5
	orr	x8, x9, x8
	mov	w10, #15
	b	LBB4_41
LBB4_32:
	lsr	x10, x1, #15
	mov	x11, #60099
	movk	x11, #62428, lsl #16
	movk	x11, #16501, lsl #32
	movk	x11, #2, lsl #48
	umulh	x10, x10, x11
	lsr	x10, x10, #20
	mov	w11, #26215
	mul	w11, w10, w11
	lsr	w11, w11, #18
	msub	w8, w11, w8, w10
	orr	w8, w8, #0x30
	strb	w8, [sp, #12]
	mov	x8, #1874919424
	movk	x8, #34546, lsl #32
	movk	x8, #35, lsl #48
	cmp	x1, x8
	b.hs	LBB4_34
	orr	x8, x9, #0x4
	mov	w10, #16
	b	LBB4_41
LBB4_34:
	mov	x8, #30807
	movk	x8, #45331, lsl #16
	movk	x8, #25903, lsl #32
	movk	x8, #14757, lsl #48
	umulh	x8, x1, x8
	lsr	x10, x8, #51
	mov	w8, #6554
	mul	w8, w10, w8
	lsr	w11, w8, #16
	mov	w8, #10
	msub	w10, w11, w8, w10
	orr	w10, w10, #0x30
	strb	w10, [sp, #11]
	mov	x10, #1569325056
	movk	x10, #17784, lsl #32
	movk	x10, #355, lsl #48
	cmp	x1, x10
	b.hs	LBB4_36
	orr	x8, x9, #0x3
	mov	w10, #17
	b	LBB4_41
LBB4_36:
	lsr	x10, x1, #17
	mov	x11, #6995
	movk	x11, #54553, lsl #16
	movk	x11, #23611, lsl #32
	umulh	x10, x10, x11
	lsr	x10, x10, #22
	mov	w11, #205
	mul	w11, w10, w11
	lsr	w11, w11, #11
	msub	w8, w11, w8, w10
	orr	w8, w8, #0x30
	strb	w8, [sp, #10]
	mov	x8, #2808348672
	movk	x8, #46771, lsl #32
	movk	x8, #3552, lsl #48
	cmp	x1, x8
	b.hs	LBB4_38
	orr	x8, x9, #0x2
	mov	w10, #18
	b	LBB4_41
LBB4_38:
	lsr	x8, x1, #18
	mov	x10, #18703
	movk	x10, #30535, lsl #16
	movk	x10, #18889, lsl #32
	umulh	x8, x8, x10
	lsr	x8, x8, #24
	sub	w10, w8, #10
	mov	x11, #2313682944
	movk	x11, #8964, lsl #32
	movk	x11, #35527, lsl #48
	cmp	x1, x11
	csel	w8, w8, w10, lo
	orr	w8, w8, #0x30
	strb	w8, [sp, #9]
	b.hs	LBB4_40
	orr	x8, x9, #0x1
	mov	w10, #19
	b	LBB4_41
LBB4_40:
	mov	w8, #49
	strb	w8, [sp, #8]
	add	x8, sp, #8
	mov	w10, #20
LBB4_41:
	ldp	x2, x9, [x0, #8]
	subs	x11, x2, x9
	csel	x11, xzr, x11, lo
	cmp	x11, x10
	csel	x10, x11, x10, lo
	adds	x19, x10, x9
	b.hs	LBB4_44
	cmp	x19, x2
	b.hi	LBB4_44
	ldr	x11, [x0]
	mov	x20, x0
	add	x0, x11, x9
	mov	x1, x8
	mov	x2, x10
	bl	_memcpy
	str	x19, [x20, #16]
	mov	x0, x20
	.cfi_def_cfa wsp, 64
	ldp	x29, x30, [sp, #48]
	ldp	x20, x19, [sp, #32]
	add	sp, sp, #64
	.cfi_def_cfa_offset 0
	.cfi_restore w30
	.cfi_restore w29
	.cfi_restore w19
	.cfi_restore w20
	ret
LBB4_44:
	.cfi_restore_state
Lloh22:
	adrp	x3, l_anon.a9251df030306a5b454b8fa8315727c3.9@PAGE
Lloh23:
	add	x3, x3, l_anon.a9251df030306a5b454b8fa8315727c3.9@PAGEOFF
	mov	x0, x9
	mov	x1, x19
	bl	__ZN4core5slice5index16slice_index_fail17hefbaa1bb9611e99cE
	.loh AdrpAdd	Lloh22, Lloh23
	.cfi_endproc

	.globl	__ZN8mcp_edge6Writer2nl17h024d6ac61767337aE
	.p2align	2
__ZN8mcp_edge6Writer2nl17h024d6ac61767337aE:
	.cfi_startproc
	stp	x20, x19, [sp, #-32]!
	.cfi_def_cfa_offset 32
	stp	x29, x30, [sp, #16]
	add	x29, sp, #16
	.cfi_def_cfa w29, 16
	.cfi_offset w30, -8
	.cfi_offset w29, -16
	.cfi_offset w19, -24
	.cfi_offset w20, -32
	.cfi_remember_state
	mov	x20, x0
	ldp	x2, x0, [x0, #8]
	cmp	x2, x0
	cset	w9, hi
	adds	x19, x0, x9
	b.hs	LBB5_3
	cmp	x19, x2
	b.hi	LBB5_3
	ldr	x10, [x20]
	add	x0, x10, x0
	mov	w1, #10
	mov	x2, x9
	bl	_memset
	str	x19, [x20, #16]
	mov	x0, x20
	.cfi_def_cfa wsp, 32
	ldp	x29, x30, [sp, #16]
	ldp	x20, x19, [sp], #32
	.cfi_def_cfa_offset 0
	.cfi_restore w30
	.cfi_restore w29
	.cfi_restore w19
	.cfi_restore w20
	ret
LBB5_3:
	.cfi_restore_state
Lloh24:
	adrp	x3, l_anon.a9251df030306a5b454b8fa8315727c3.9@PAGE
Lloh25:
	add	x3, x3, l_anon.a9251df030306a5b454b8fa8315727c3.9@PAGEOFF
	mov	x1, x19
	bl	__ZN4core5slice5index16slice_index_fail17hefbaa1bb9611e99cE
	.loh AdrpAdd	Lloh24, Lloh25
	.cfi_endproc

	.globl	__ZN8mcp_edge6Writer3esc17h5b0ab0e38a6fb632E
	.p2align	2
__ZN8mcp_edge6Writer3esc17h5b0ab0e38a6fb632E:
	.cfi_startproc
	sub	sp, sp, #112
	.cfi_def_cfa_offset 112
	stp	x28, x27, [sp, #16]
	stp	x26, x25, [sp, #32]
	stp	x24, x23, [sp, #48]
	stp	x22, x21, [sp, #64]
	stp	x20, x19, [sp, #80]
	stp	x29, x30, [sp, #96]
	add	x29, sp, #96
	.cfi_def_cfa w29, 16
	.cfi_offset w30, -8
	.cfi_offset w29, -16
	.cfi_offset w19, -24
	.cfi_offset w20, -32
	.cfi_offset w21, -40
	.cfi_offset w22, -48
	.cfi_offset w23, -56
	.cfi_offset w24, -64
	.cfi_offset w25, -72
	.cfi_offset w26, -80
	.cfi_offset w27, -88
	.cfi_offset w28, -96
	.cfi_remember_state
	mov	x21, x0
	cbz	x2, LBB6_26
	mov	x20, x2
	mov	x22, x1
	ldp	x28, x19, [x21]
Lloh26:
	adrp	x24, l_anon.a9251df030306a5b454b8fa8315727c3.6@PAGE
Lloh27:
	add	x24, x24, l_anon.a9251df030306a5b454b8fa8315727c3.6@PAGEOFF
	mov	w23, #2
	ldr	x0, [x21, #16]
Lloh28:
	adrp	x25, l_anon.a9251df030306a5b454b8fa8315727c3.7@PAGE
Lloh29:
	add	x25, x25, l_anon.a9251df030306a5b454b8fa8315727c3.7@PAGEOFF
Lloh30:
	adrp	x26, l_anon.a9251df030306a5b454b8fa8315727c3.5@PAGE
Lloh31:
	add	x26, x26, l_anon.a9251df030306a5b454b8fa8315727c3.5@PAGEOFF
	b	LBB6_3
LBB6_2:
	add	x0, x28, x0
Lloh32:
	adrp	x1, l_anon.a9251df030306a5b454b8fa8315727c3.8@PAGE
Lloh33:
	add	x1, x1, l_anon.a9251df030306a5b454b8fa8315727c3.8@PAGEOFF
	bl	_memcpy
	str	x27, [x21, #16]
	mov	x0, x27
	subs	x20, x20, #1
	b.eq	LBB6_26
LBB6_3:
	ldrb	w8, [x22], #1
	strb	w8, [sp, #15]
	cmp	w8, #12
	b.le	LBB6_10
	cmp	w8, #13
	b.eq	LBB6_15
	cmp	w8, #34
	b.eq	LBB6_18
	cmp	w8, #92
	b.ne	LBB6_23
	subs	x8, x19, x0
	csel	x8, xzr, x8, lo
	cmp	x8, #2
	csel	x2, x8, x23, lo
	adds	x27, x2, x0
	b.hs	LBB6_27
	cmp	x27, x19
	b.hi	LBB6_27
	add	x0, x28, x0
	mov	w1, #92
	bl	_memset
	str	x27, [x21, #16]
	mov	x0, x27
	subs	x20, x20, #1
	b.ne	LBB6_3
	b	LBB6_26
LBB6_10:
	cmp	w8, #9
	b.eq	LBB6_21
	cmp	w8, #10
	b.ne	LBB6_23
	subs	x8, x19, x0
	csel	x8, xzr, x8, lo
	cmp	x8, #2
	csel	x2, x8, x23, lo
	adds	x27, x2, x0
	b.hs	LBB6_27
	cmp	x27, x19
	b.hi	LBB6_27
	add	x0, x28, x0
	mov	x1, x24
	bl	_memcpy
	str	x27, [x21, #16]
	mov	x0, x27
	subs	x20, x20, #1
	b.ne	LBB6_3
	b	LBB6_26
LBB6_15:
	subs	x8, x19, x0
	csel	x8, xzr, x8, lo
	cmp	x8, #2
	csel	x2, x8, x23, lo
	adds	x27, x2, x0
	b.hs	LBB6_27
	cmp	x27, x19
	b.hi	LBB6_27
	add	x0, x28, x0
	mov	x1, x25
	bl	_memcpy
	str	x27, [x21, #16]
	mov	x0, x27
	subs	x20, x20, #1
	b.ne	LBB6_3
	b	LBB6_26
LBB6_18:
	subs	x8, x19, x0
	csel	x8, xzr, x8, lo
	cmp	x8, #2
	csel	x2, x8, x23, lo
	adds	x27, x2, x0
	b.hs	LBB6_27
	cmp	x27, x19
	b.hi	LBB6_27
	add	x0, x28, x0
	mov	x1, x26
	bl	_memcpy
	str	x27, [x21, #16]
	mov	x0, x27
	subs	x20, x20, #1
	b.ne	LBB6_3
	b	LBB6_26
LBB6_21:
	subs	x8, x19, x0
	csel	x8, xzr, x8, lo
	cmp	x8, #2
	csel	x2, x8, x23, lo
	adds	x27, x2, x0
	b.hs	LBB6_27
	cmp	x27, x19
	b.ls	LBB6_2
	b	LBB6_27
LBB6_23:
	cmp	x19, x0
	cset	w2, hi
	adds	x27, x0, x2
	b.hs	LBB6_27
	cmp	x27, x19
	b.hi	LBB6_27
	add	x0, x28, x0
	add	x1, sp, #15
	bl	_memcpy
	str	x27, [x21, #16]
	mov	x0, x27
	subs	x20, x20, #1
	b.ne	LBB6_3
LBB6_26:
	mov	x0, x21
	.cfi_def_cfa wsp, 112
	ldp	x29, x30, [sp, #96]
	ldp	x20, x19, [sp, #80]
	ldp	x22, x21, [sp, #64]
	ldp	x24, x23, [sp, #48]
	ldp	x26, x25, [sp, #32]
	ldp	x28, x27, [sp, #16]
	add	sp, sp, #112
	.cfi_def_cfa_offset 0
	.cfi_restore w30
	.cfi_restore w29
	.cfi_restore w19
	.cfi_restore w20
	.cfi_restore w21
	.cfi_restore w22
	.cfi_restore w23
	.cfi_restore w24
	.cfi_restore w25
	.cfi_restore w26
	.cfi_restore w27
	.cfi_restore w28
	ret
LBB6_27:
	.cfi_restore_state
Lloh34:
	adrp	x3, l_anon.a9251df030306a5b454b8fa8315727c3.9@PAGE
Lloh35:
	add	x3, x3, l_anon.a9251df030306a5b454b8fa8315727c3.9@PAGEOFF
	mov	x1, x27
	mov	x2, x19
	bl	__ZN4core5slice5index16slice_index_fail17hefbaa1bb9611e99cE
	.loh AdrpAdd	Lloh30, Lloh31
	.loh AdrpAdd	Lloh28, Lloh29
	.loh AdrpAdd	Lloh26, Lloh27
	.loh AdrpAdd	Lloh32, Lloh33
	.loh AdrpAdd	Lloh34, Lloh35
	.cfi_endproc

	.globl	__ZN8mcp_edge7rpc_err17hf3842a3b76b72794E
	.p2align	2
__ZN8mcp_edge7rpc_err17hf3842a3b76b72794E:
	.cfi_startproc
	stp	x26, x25, [sp, #-80]!
	.cfi_def_cfa_offset 80
	stp	x24, x23, [sp, #16]
	stp	x22, x21, [sp, #32]
	stp	x20, x19, [sp, #48]
	stp	x29, x30, [sp, #64]
	add	x29, sp, #64
	.cfi_def_cfa w29, 16
	.cfi_offset w30, -8
	.cfi_offset w29, -16
	.cfi_offset w19, -24
	.cfi_offset w20, -32
	.cfi_offset w21, -40
	.cfi_offset w22, -48
	.cfi_offset w23, -56
	.cfi_offset w24, -64
	.cfi_offset w25, -72
	.cfi_offset w26, -80
	.cfi_remember_state
	mov	x22, x2
	mov	x19, x0
	ldp	x2, x0, [x0, #8]
	subs	x8, x2, x0
	csel	x8, xzr, x8, lo
	mov	w9, #22
	cmp	x8, #22
	csel	x8, x8, x9, lo
	adds	x24, x8, x0
	b.hs	LBB7_15
	cmp	x24, x2
	b.hi	LBB7_15
	mov	x20, x4
	mov	x21, x3
	mov	x23, x1
	ldr	x9, [x19]
Lloh36:
	adrp	x1, l_anon.a9251df030306a5b454b8fa8315727c3.12@PAGE
Lloh37:
	add	x1, x1, l_anon.a9251df030306a5b454b8fa8315727c3.12@PAGEOFF
	add	x0, x9, x0
	mov	x2, x8
	bl	_memcpy
	str	x24, [x19, #16]
	mov	x0, x19
	mov	x1, x23
	bl	__ZN8mcp_edge6Writer1u17h01d45ee9b9ccbe60E
	ldp	x23, x0, [x19, #8]
	subs	x8, x23, x0
	csel	x8, xzr, x8, lo
	mov	w9, #17
	cmp	x8, #17
	csel	x2, x8, x9, lo
	adds	x24, x2, x0
	b.hs	LBB7_16
	cmp	x24, x23
	b.hi	LBB7_16
	ldr	x26, [x19]
Lloh38:
	adrp	x1, l_anon.a9251df030306a5b454b8fa8315727c3.13@PAGE
Lloh39:
	add	x1, x1, l_anon.a9251df030306a5b454b8fa8315727c3.13@PAGEOFF
	add	x0, x26, x0
	bl	_memcpy
	str	x24, [x19, #16]
	tbz	w22, #31, LBB7_8
	cmp	x23, x24
	cset	w2, hi
	adds	x25, x24, x2
	b.hs	LBB7_20
	cmp	x25, x23
	b.hi	LBB7_20
	add	x0, x26, x24
	mov	w1, #45
	bl	_memset
	str	x25, [x19, #16]
	neg	w22, w22
LBB7_8:
	mov	w1, w22
	mov	x0, x19
	bl	__ZN8mcp_edge6Writer1u17h01d45ee9b9ccbe60E
	ldp	x2, x0, [x19, #8]
	subs	x8, x2, x0
	csel	x8, xzr, x8, lo
	mov	w9, #12
	cmp	x8, #12
	csel	x8, x8, x9, lo
	adds	x22, x8, x0
	b.hs	LBB7_17
	cmp	x22, x2
	b.hi	LBB7_17
	ldr	x9, [x19]
Lloh40:
	adrp	x1, l_anon.a9251df030306a5b454b8fa8315727c3.14@PAGE
Lloh41:
	add	x1, x1, l_anon.a9251df030306a5b454b8fa8315727c3.14@PAGEOFF
	add	x0, x9, x0
	mov	x2, x8
	bl	_memcpy
	str	x22, [x19, #16]
	mov	x0, x19
	mov	x1, x21
	mov	x2, x20
	bl	__ZN8mcp_edge6Writer3esc17h5b0ab0e38a6fb632E
	ldp	x20, x0, [x19, #8]
	subs	x8, x20, x0
	csel	x8, xzr, x8, lo
	mov	w9, #3
	cmp	x8, #3
	csel	x2, x8, x9, lo
	adds	x21, x2, x0
	b.hs	LBB7_18
	cmp	x21, x20
	b.hi	LBB7_18
	ldr	x23, [x19]
Lloh42:
	adrp	x1, l_anon.a9251df030306a5b454b8fa8315727c3.15@PAGE
Lloh43:
	add	x1, x1, l_anon.a9251df030306a5b454b8fa8315727c3.15@PAGEOFF
	add	x0, x23, x0
	bl	_memcpy
	str	x21, [x19, #16]
	cmp	x20, x21
	cset	w2, hi
	adds	x22, x21, x2
	b.hs	LBB7_19
	cmp	x22, x20
	b.hi	LBB7_19
	add	x0, x23, x21
	mov	w1, #10
	bl	_memset
	str	x22, [x19, #16]
	.cfi_def_cfa wsp, 80
	ldp	x29, x30, [sp, #64]
	ldp	x20, x19, [sp, #48]
	ldp	x22, x21, [sp, #32]
	ldp	x24, x23, [sp, #16]
	ldp	x26, x25, [sp], #80
	.cfi_def_cfa_offset 0
	.cfi_restore w30
	.cfi_restore w29
	.cfi_restore w19
	.cfi_restore w20
	.cfi_restore w21
	.cfi_restore w22
	.cfi_restore w23
	.cfi_restore w24
	.cfi_restore w25
	.cfi_restore w26
	ret
LBB7_15:
	.cfi_restore_state
Lloh44:
	adrp	x3, l_anon.a9251df030306a5b454b8fa8315727c3.9@PAGE
Lloh45:
	add	x3, x3, l_anon.a9251df030306a5b454b8fa8315727c3.9@PAGEOFF
	mov	x1, x24
	bl	__ZN4core5slice5index16slice_index_fail17hefbaa1bb9611e99cE
LBB7_16:
Lloh46:
	adrp	x3, l_anon.a9251df030306a5b454b8fa8315727c3.9@PAGE
Lloh47:
	add	x3, x3, l_anon.a9251df030306a5b454b8fa8315727c3.9@PAGEOFF
	mov	x1, x24
	mov	x2, x23
	bl	__ZN4core5slice5index16slice_index_fail17hefbaa1bb9611e99cE
LBB7_17:
Lloh48:
	adrp	x3, l_anon.a9251df030306a5b454b8fa8315727c3.9@PAGE
Lloh49:
	add	x3, x3, l_anon.a9251df030306a5b454b8fa8315727c3.9@PAGEOFF
	mov	x1, x22
	bl	__ZN4core5slice5index16slice_index_fail17hefbaa1bb9611e99cE
LBB7_18:
Lloh50:
	adrp	x3, l_anon.a9251df030306a5b454b8fa8315727c3.9@PAGE
Lloh51:
	add	x3, x3, l_anon.a9251df030306a5b454b8fa8315727c3.9@PAGEOFF
	mov	x1, x21
	mov	x2, x20
	bl	__ZN4core5slice5index16slice_index_fail17hefbaa1bb9611e99cE
LBB7_19:
Lloh52:
	adrp	x3, l_anon.a9251df030306a5b454b8fa8315727c3.9@PAGE
Lloh53:
	add	x3, x3, l_anon.a9251df030306a5b454b8fa8315727c3.9@PAGEOFF
	mov	x0, x21
	mov	x1, x22
	mov	x2, x20
	bl	__ZN4core5slice5index16slice_index_fail17hefbaa1bb9611e99cE
LBB7_20:
Lloh54:
	adrp	x3, l_anon.a9251df030306a5b454b8fa8315727c3.9@PAGE
Lloh55:
	add	x3, x3, l_anon.a9251df030306a5b454b8fa8315727c3.9@PAGEOFF
	mov	x0, x24
	mov	x1, x25
	mov	x2, x23
	bl	__ZN4core5slice5index16slice_index_fail17hefbaa1bb9611e99cE
	.loh AdrpAdd	Lloh36, Lloh37
	.loh AdrpAdd	Lloh38, Lloh39
	.loh AdrpAdd	Lloh40, Lloh41
	.loh AdrpAdd	Lloh42, Lloh43
	.loh AdrpAdd	Lloh44, Lloh45
	.loh AdrpAdd	Lloh46, Lloh47
	.loh AdrpAdd	Lloh48, Lloh49
	.loh AdrpAdd	Lloh50, Lloh51
	.loh AdrpAdd	Lloh52, Lloh53
	.loh AdrpAdd	Lloh54, Lloh55
	.cfi_endproc

	.globl	__ZN8mcp_edge9transport13UnixTransport3new17h3ae3cdf55fabde02E
	.p2align	2
__ZN8mcp_edge9transport13UnixTransport3new17h3ae3cdf55fabde02E:
Lfunc_begin0:
	.cfi_startproc
	.cfi_personality 155, _rust_eh_personality
	.cfi_lsda 16, Lexception0
	stp	x24, x23, [sp, #-64]!
	.cfi_def_cfa_offset 64
	stp	x22, x21, [sp, #16]
	stp	x20, x19, [sp, #32]
	stp	x29, x30, [sp, #48]
	add	x29, sp, #48
	.cfi_def_cfa w29, 16
	.cfi_offset w30, -8
	.cfi_offset w29, -16
	.cfi_offset w19, -24
	.cfi_offset w20, -32
	.cfi_offset w21, -40
	.cfi_offset w22, -48
	.cfi_offset w23, -56
	.cfi_offset w24, -64
	.cfi_remember_state
	mov	x20, x1
	mov	x21, x0
	bl	__ZN3std3sys2fs11remove_file17h7e1e3b5edf9300cfE
	and	x8, x0, #0x3
	cmp	x8, #1
	b.eq	LBB8_2
LBB8_1:
	mov	x0, x21
	mov	x1, x20
	.cfi_def_cfa wsp, 64
	ldp	x29, x30, [sp, #48]
	ldp	x20, x19, [sp, #32]
	ldp	x22, x21, [sp, #16]
	ldp	x24, x23, [sp], #64
	.cfi_def_cfa_offset 0
	.cfi_restore w30
	.cfi_restore w29
	.cfi_restore w19
	.cfi_restore w20
	.cfi_restore w21
	.cfi_restore w22
	.cfi_restore w23
	.cfi_restore w24
	ret
LBB8_2:
	.cfi_restore_state
	mov	x19, x0
	ldr	x22, [x19, #-1]!
	ldr	x23, [x19, #8]
	ldr	x8, [x23]
	cbz	x8, LBB8_4
Ltmp0:
	mov	x0, x22
	blr	x8
Ltmp1:
LBB8_4:
	ldr	x1, [x23, #8]
	cbz	x1, LBB8_6
	ldr	x2, [x23, #16]
	mov	x0, x22
	bl	__RNvCsiGVaDesi5rv_7___rustc14___rust_dealloc
LBB8_6:
	mov	x0, x19
	mov	w1, #24
	mov	w2, #8
	bl	__RNvCsiGVaDesi5rv_7___rustc14___rust_dealloc
	b	LBB8_1
LBB8_7:
Ltmp2:
	mov	x20, x0
	ldr	x1, [x23, #8]
	cbz	x1, LBB8_9
	ldr	x2, [x23, #16]
	mov	x0, x22
	bl	__RNvCsiGVaDesi5rv_7___rustc14___rust_dealloc
LBB8_9:
	mov	x0, x19
	mov	w1, #24
	mov	w2, #8
	bl	__RNvCsiGVaDesi5rv_7___rustc14___rust_dealloc
	mov	x0, x20
	bl	__Unwind_Resume
Lfunc_end0:
	.cfi_endproc
	.section	__TEXT,__gcc_except_tab
	.p2align	2, 0x0
GCC_except_table8:
Lexception0:
	.byte	255
	.byte	255
	.byte	1
	.uleb128 Lcst_end0-Lcst_begin0
Lcst_begin0:
	.uleb128 Lfunc_begin0-Lfunc_begin0
	.uleb128 Ltmp0-Lfunc_begin0
	.byte	0
	.byte	0
	.uleb128 Ltmp0-Lfunc_begin0
	.uleb128 Ltmp1-Ltmp0
	.uleb128 Ltmp2-Lfunc_begin0
	.byte	0
	.uleb128 Ltmp1-Lfunc_begin0
	.uleb128 Lfunc_end0-Ltmp1
	.byte	0
	.byte	0
Lcst_end0:
	.p2align	2, 0x0

	.section	__TEXT,__cstring,cstring_literals
l_anon.a9251df030306a5b454b8fa8315727c3.0:
	.asciz	"src/lib.rs"

	.section	__DATA,__const
	.p2align	3, 0x0
l_anon.a9251df030306a5b454b8fa8315727c3.1:
	.quad	l_anon.a9251df030306a5b454b8fa8315727c3.0
	.asciz	"\n\000\000\000\000\000\000\000\016\001\000\000\033\000\000"

	.p2align	3, 0x0
l_anon.a9251df030306a5b454b8fa8315727c3.2:
	.quad	l_anon.a9251df030306a5b454b8fa8315727c3.0
	.asciz	"\n\000\000\000\000\000\000\000\362\000\000\000\033\000\000"

	.p2align	3, 0x0
l_anon.a9251df030306a5b454b8fa8315727c3.3:
	.quad	l_anon.a9251df030306a5b454b8fa8315727c3.0
	.asciz	"\n\000\000\000\000\000\000\000\006\001\000\000\024\000\000"

	.p2align	3, 0x0
l_anon.a9251df030306a5b454b8fa8315727c3.4:
	.quad	l_anon.a9251df030306a5b454b8fa8315727c3.0
	.asciz	"\n\000\000\000\000\000\000\000\002\001\000\000\033\000\000"

	.section	__TEXT,__const
l_anon.a9251df030306a5b454b8fa8315727c3.5:
	.ascii	"\\\""

l_anon.a9251df030306a5b454b8fa8315727c3.6:
	.ascii	"\\n"

l_anon.a9251df030306a5b454b8fa8315727c3.7:
	.ascii	"\\r"

l_anon.a9251df030306a5b454b8fa8315727c3.8:
	.ascii	"\\t"

	.section	__DATA,__const
	.p2align	3, 0x0
l_anon.a9251df030306a5b454b8fa8315727c3.9:
	.quad	l_anon.a9251df030306a5b454b8fa8315727c3.0
	.asciz	"\n\000\000\000\000\000\000\000\300\000\000\000\021\000\000"

	.section	__TEXT,__const
l_anon.a9251df030306a5b454b8fa8315727c3.10:
	.ascii	"window size must be non-zero"

	.section	__DATA,__const
	.p2align	3, 0x0
l_anon.a9251df030306a5b454b8fa8315727c3.11:
	.quad	l_anon.a9251df030306a5b454b8fa8315727c3.0
	.asciz	"\n\000\000\000\000\000\000\000%\001\000\000\016\000\000"

	.section	__TEXT,__const
l_anon.a9251df030306a5b454b8fa8315727c3.12:
	.ascii	"{\"jsonrpc\":\"2.0\",\"id\":"

l_anon.a9251df030306a5b454b8fa8315727c3.13:
	.ascii	",\"error\":{\"code\":"

l_anon.a9251df030306a5b454b8fa8315727c3.14:
	.ascii	",\"message\":\""

l_anon.a9251df030306a5b454b8fa8315727c3.15:
	.ascii	"\"}}"

.subsections_via_symbols
