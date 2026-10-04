//! SMOL-V -> SPIR-V decoder (Unity stores Vulkan shader programs as SMOL-V).
//! Port of `smolv::Decode` from https://github.com/aras-p/smol-v (MIT / public domain,
//! (c) 2016-2024 Aras Pranckevicius); behaviour and tables follow that file exactly.

const SPIRV_MAGIC: u32 = 0x0723_0203;
const SMOL_MAGIC: u32 = 0x534D_4F4C;

// SPIR-V opcodes used by the encoding
const OP_DECORATE: u32 = 71;
const OP_MEMBER_DECORATE: u32 = 72;
const OP_VECTOR_SHUFFLE: u32 = 79;
const OP_VECTOR_SHUFFLE_COMPACT: u32 = 13; // SMOL-V only
const OP_LOAD: u32 = 61;
const OP_ACCESS_CHAIN: u32 = 65;
const OP_MODULE_PROCESSED: u32 = 330;

/// Per opcode: [hasResult, hasType, deltaFromResult, varrest] (kSpirvOpData).
#[rustfmt::skip]
static OP_DATA: [[u8; 4]; 367] = [
    [0, 0, 0, 0], // Nop
    [1, 1, 0, 0], // Undef
    [0, 0, 0, 0], // SourceContinued
    [0, 0, 0, 1], // Source
    [0, 0, 0, 0], // SourceExtension
    [0, 0, 0, 0], // Name
    [0, 0, 0, 0], // MemberName
    [0, 0, 0, 0], // String
    [0, 0, 0, 1], // Line
    [1, 1, 0, 0], // #9
    [0, 0, 0, 0], // Extension
    [1, 0, 0, 0], // ExtInstImport
    [1, 1, 0, 1], // ExtInst
    [1, 1, 2, 1], // VectorShuffleCompact - new in SMOLV
    [0, 0, 0, 1], // MemoryModel
    [0, 0, 0, 1], // EntryPoint
    [0, 0, 0, 1], // ExecutionMode
    [0, 0, 0, 1], // Capability
    [1, 1, 0, 0], // #18
    [1, 0, 0, 1], // TypeVoid
    [1, 0, 0, 1], // TypeBool
    [1, 0, 0, 1], // TypeInt
    [1, 0, 0, 1], // TypeFloat
    [1, 0, 0, 1], // TypeVector
    [1, 0, 0, 1], // TypeMatrix
    [1, 0, 0, 1], // TypeImage
    [1, 0, 0, 1], // TypeSampler
    [1, 0, 0, 1], // TypeSampledImage
    [1, 0, 0, 1], // TypeArray
    [1, 0, 0, 1], // TypeRuntimeArray
    [1, 0, 0, 1], // TypeStruct
    [1, 0, 0, 1], // TypeOpaque
    [1, 0, 0, 1], // TypePointer
    [1, 0, 0, 1], // TypeFunction
    [1, 0, 0, 1], // TypeEvent
    [1, 0, 0, 1], // TypeDeviceEvent
    [1, 0, 0, 1], // TypeReserveId
    [1, 0, 0, 1], // TypeQueue
    [1, 0, 0, 1], // TypePipe
    [0, 0, 0, 1], // TypeForwardPointer
    [1, 1, 0, 0], // #40
    [1, 1, 0, 0], // ConstantTrue
    [1, 1, 0, 0], // ConstantFalse
    [1, 1, 0, 0], // Constant
    [1, 1, 9, 0], // ConstantComposite
    [1, 1, 0, 1], // ConstantSampler
    [1, 1, 0, 0], // ConstantNull
    [1, 1, 0, 0], // #47
    [1, 1, 0, 0], // SpecConstantTrue
    [1, 1, 0, 0], // SpecConstantFalse
    [1, 1, 0, 0], // SpecConstant
    [1, 1, 9, 0], // SpecConstantComposite
    [1, 1, 0, 0], // SpecConstantOp
    [1, 1, 0, 0], // #53
    [1, 1, 0, 1], // Function
    [1, 1, 0, 0], // FunctionParameter
    [0, 0, 0, 0], // FunctionEnd
    [1, 1, 9, 0], // FunctionCall
    [1, 1, 0, 0], // #58
    [1, 1, 0, 1], // Variable
    [1, 1, 0, 0], // ImageTexelPointer
    [1, 1, 1, 1], // Load
    [0, 0, 2, 1], // Store
    [0, 0, 0, 0], // CopyMemory
    [0, 0, 0, 0], // CopyMemorySized
    [1, 1, 0, 1], // AccessChain
    [1, 1, 0, 0], // InBoundsAccessChain
    [1, 1, 0, 0], // PtrAccessChain
    [1, 1, 0, 0], // ArrayLength
    [1, 1, 0, 0], // GenericPtrMemSemantics
    [1, 1, 0, 0], // InBoundsPtrAccessChain
    [0, 0, 0, 1], // Decorate
    [0, 0, 0, 1], // MemberDecorate
    [1, 0, 0, 0], // DecorationGroup
    [0, 0, 0, 0], // GroupDecorate
    [0, 0, 0, 0], // GroupMemberDecorate
    [1, 1, 0, 0], // #76
    [1, 1, 1, 1], // VectorExtractDynamic
    [1, 1, 2, 1], // VectorInsertDynamic
    [1, 1, 2, 1], // VectorShuffle
    [1, 1, 9, 0], // CompositeConstruct
    [1, 1, 1, 1], // CompositeExtract
    [1, 1, 2, 1], // CompositeInsert
    [1, 1, 1, 0], // CopyObject
    [1, 1, 0, 0], // Transpose
    [1, 1, 0, 0], // #85
    [1, 1, 0, 0], // SampledImage
    [1, 1, 2, 1], // ImageSampleImplicitLod
    [1, 1, 2, 1], // ImageSampleExplicitLod
    [1, 1, 3, 1], // ImageSampleDrefImplicitLod
    [1, 1, 3, 1], // ImageSampleDrefExplicitLod
    [1, 1, 2, 1], // ImageSampleProjImplicitLod
    [1, 1, 2, 1], // ImageSampleProjExplicitLod
    [1, 1, 3, 1], // ImageSampleProjDrefImplicitLod
    [1, 1, 3, 1], // ImageSampleProjDrefExplicitLod
    [1, 1, 2, 1], // ImageFetch
    [1, 1, 3, 1], // ImageGather
    [1, 1, 3, 1], // ImageDrefGather
    [1, 1, 2, 1], // ImageRead
    [0, 0, 3, 1], // ImageWrite
    [1, 1, 1, 0], // Image
    [1, 1, 1, 0], // ImageQueryFormat
    [1, 1, 1, 0], // ImageQueryOrder
    [1, 1, 2, 0], // ImageQuerySizeLod
    [1, 1, 1, 0], // ImageQuerySize
    [1, 1, 2, 0], // ImageQueryLod
    [1, 1, 1, 0], // ImageQueryLevels
    [1, 1, 1, 0], // ImageQuerySamples
    [1, 1, 0, 0], // #108
    [1, 1, 1, 0], // ConvertFToU
    [1, 1, 1, 0], // ConvertFToS
    [1, 1, 1, 0], // ConvertSToF
    [1, 1, 1, 0], // ConvertUToF
    [1, 1, 1, 0], // UConvert
    [1, 1, 1, 0], // SConvert
    [1, 1, 1, 0], // FConvert
    [1, 1, 1, 0], // QuantizeToF16
    [1, 1, 1, 0], // ConvertPtrToU
    [1, 1, 1, 0], // SatConvertSToU
    [1, 1, 1, 0], // SatConvertUToS
    [1, 1, 1, 0], // ConvertUToPtr
    [1, 1, 1, 0], // PtrCastToGeneric
    [1, 1, 1, 0], // GenericCastToPtr
    [1, 1, 1, 1], // GenericCastToPtrExplicit
    [1, 1, 1, 0], // Bitcast
    [1, 1, 0, 0], // #125
    [1, 1, 1, 0], // SNegate
    [1, 1, 1, 0], // FNegate
    [1, 1, 2, 0], // IAdd
    [1, 1, 2, 0], // FAdd
    [1, 1, 2, 0], // ISub
    [1, 1, 2, 0], // FSub
    [1, 1, 2, 0], // IMul
    [1, 1, 2, 0], // FMul
    [1, 1, 2, 0], // UDiv
    [1, 1, 2, 0], // SDiv
    [1, 1, 2, 0], // FDiv
    [1, 1, 2, 0], // UMod
    [1, 1, 2, 0], // SRem
    [1, 1, 2, 0], // SMod
    [1, 1, 2, 0], // FRem
    [1, 1, 2, 0], // FMod
    [1, 1, 2, 0], // VectorTimesScalar
    [1, 1, 2, 0], // MatrixTimesScalar
    [1, 1, 2, 0], // VectorTimesMatrix
    [1, 1, 2, 0], // MatrixTimesVector
    [1, 1, 2, 0], // MatrixTimesMatrix
    [1, 1, 2, 0], // OuterProduct
    [1, 1, 2, 0], // Dot
    [1, 1, 2, 0], // IAddCarry
    [1, 1, 2, 0], // ISubBorrow
    [1, 1, 2, 0], // UMulExtended
    [1, 1, 2, 0], // SMulExtended
    [1, 1, 0, 0], // #153
    [1, 1, 1, 0], // Any
    [1, 1, 1, 0], // All
    [1, 1, 1, 0], // IsNan
    [1, 1, 1, 0], // IsInf
    [1, 1, 1, 0], // IsFinite
    [1, 1, 1, 0], // IsNormal
    [1, 1, 1, 0], // SignBitSet
    [1, 1, 2, 0], // LessOrGreater
    [1, 1, 2, 0], // Ordered
    [1, 1, 2, 0], // Unordered
    [1, 1, 2, 0], // LogicalEqual
    [1, 1, 2, 0], // LogicalNotEqual
    [1, 1, 2, 0], // LogicalOr
    [1, 1, 2, 0], // LogicalAnd
    [1, 1, 1, 0], // LogicalNot
    [1, 1, 3, 0], // Select
    [1, 1, 2, 0], // IEqual
    [1, 1, 2, 0], // INotEqual
    [1, 1, 2, 0], // UGreaterThan
    [1, 1, 2, 0], // SGreaterThan
    [1, 1, 2, 0], // UGreaterThanEqual
    [1, 1, 2, 0], // SGreaterThanEqual
    [1, 1, 2, 0], // ULessThan
    [1, 1, 2, 0], // SLessThan
    [1, 1, 2, 0], // ULessThanEqual
    [1, 1, 2, 0], // SLessThanEqual
    [1, 1, 2, 0], // FOrdEqual
    [1, 1, 2, 0], // FUnordEqual
    [1, 1, 2, 0], // FOrdNotEqual
    [1, 1, 2, 0], // FUnordNotEqual
    [1, 1, 2, 0], // FOrdLessThan
    [1, 1, 2, 0], // FUnordLessThan
    [1, 1, 2, 0], // FOrdGreaterThan
    [1, 1, 2, 0], // FUnordGreaterThan
    [1, 1, 2, 0], // FOrdLessThanEqual
    [1, 1, 2, 0], // FUnordLessThanEqual
    [1, 1, 2, 0], // FOrdGreaterThanEqual
    [1, 1, 2, 0], // FUnordGreaterThanEqual
    [1, 1, 0, 0], // #192
    [1, 1, 0, 0], // #193
    [1, 1, 2, 0], // ShiftRightLogical
    [1, 1, 2, 0], // ShiftRightArithmetic
    [1, 1, 2, 0], // ShiftLeftLogical
    [1, 1, 2, 0], // BitwiseOr
    [1, 1, 2, 0], // BitwiseXor
    [1, 1, 2, 0], // BitwiseAnd
    [1, 1, 1, 0], // Not
    [1, 1, 4, 0], // BitFieldInsert
    [1, 1, 3, 0], // BitFieldSExtract
    [1, 1, 3, 0], // BitFieldUExtract
    [1, 1, 1, 0], // BitReverse
    [1, 1, 1, 0], // BitCount
    [1, 1, 0, 0], // #206
    [1, 1, 0, 0], // DPdx
    [1, 1, 0, 0], // DPdy
    [1, 1, 0, 0], // Fwidth
    [1, 1, 0, 0], // DPdxFine
    [1, 1, 0, 0], // DPdyFine
    [1, 1, 0, 0], // FwidthFine
    [1, 1, 0, 0], // DPdxCoarse
    [1, 1, 0, 0], // DPdyCoarse
    [1, 1, 0, 0], // FwidthCoarse
    [1, 1, 0, 0], // #216
    [1, 1, 0, 0], // #217
    [0, 0, 0, 0], // EmitVertex
    [0, 0, 0, 0], // EndPrimitive
    [0, 0, 0, 0], // EmitStreamVertex
    [0, 0, 0, 0], // EndStreamPrimitive
    [1, 1, 0, 0], // #222
    [1, 1, 0, 0], // #223
    [0, 0, 3, 0], // ControlBarrier
    [0, 0, 2, 0], // MemoryBarrier
    [1, 1, 0, 0], // #226
    [1, 1, 0, 0], // AtomicLoad
    [0, 0, 0, 0], // AtomicStore
    [1, 1, 0, 0], // AtomicExchange
    [1, 1, 0, 0], // AtomicCompareExchange
    [1, 1, 0, 0], // AtomicCompareExchangeWeak
    [1, 1, 0, 0], // AtomicIIncrement
    [1, 1, 0, 0], // AtomicIDecrement
    [1, 1, 0, 0], // AtomicIAdd
    [1, 1, 0, 0], // AtomicISub
    [1, 1, 0, 0], // AtomicSMin
    [1, 1, 0, 0], // AtomicUMin
    [1, 1, 0, 0], // AtomicSMax
    [1, 1, 0, 0], // AtomicUMax
    [1, 1, 0, 0], // AtomicAnd
    [1, 1, 0, 0], // AtomicOr
    [1, 1, 0, 0], // AtomicXor
    [1, 1, 0, 0], // #243
    [1, 1, 0, 0], // #244
    [1, 1, 0, 0], // Phi
    [0, 0, 2, 1], // LoopMerge
    [0, 0, 1, 1], // SelectionMerge
    [1, 0, 0, 0], // Label
    [0, 0, 1, 0], // Branch
    [0, 0, 3, 1], // BranchConditional
    [0, 0, 0, 0], // Switch
    [0, 0, 0, 0], // Kill
    [0, 0, 0, 0], // Return
    [0, 0, 0, 0], // ReturnValue
    [0, 0, 0, 0], // Unreachable
    [0, 0, 0, 0], // LifetimeStart
    [0, 0, 0, 0], // LifetimeStop
    [1, 1, 0, 0], // #258
    [1, 1, 0, 0], // GroupAsyncCopy
    [0, 0, 0, 0], // GroupWaitEvents
    [1, 1, 0, 0], // GroupAll
    [1, 1, 0, 0], // GroupAny
    [1, 1, 0, 0], // GroupBroadcast
    [1, 1, 0, 0], // GroupIAdd
    [1, 1, 0, 0], // GroupFAdd
    [1, 1, 0, 0], // GroupFMin
    [1, 1, 0, 0], // GroupUMin
    [1, 1, 0, 0], // GroupSMin
    [1, 1, 0, 0], // GroupFMax
    [1, 1, 0, 0], // GroupUMax
    [1, 1, 0, 0], // GroupSMax
    [1, 1, 0, 0], // #272
    [1, 1, 0, 0], // #273
    [1, 1, 0, 0], // ReadPipe
    [1, 1, 0, 0], // WritePipe
    [1, 1, 0, 0], // ReservedReadPipe
    [1, 1, 0, 0], // ReservedWritePipe
    [1, 1, 0, 0], // ReserveReadPipePackets
    [1, 1, 0, 0], // ReserveWritePipePackets
    [0, 0, 0, 0], // CommitReadPipe
    [0, 0, 0, 0], // CommitWritePipe
    [1, 1, 0, 0], // IsValidReserveId
    [1, 1, 0, 0], // GetNumPipePackets
    [1, 1, 0, 0], // GetMaxPipePackets
    [1, 1, 0, 0], // GroupReserveReadPipePackets
    [1, 1, 0, 0], // GroupReserveWritePipePackets
    [0, 0, 0, 0], // GroupCommitReadPipe
    [0, 0, 0, 0], // GroupCommitWritePipe
    [1, 1, 0, 0], // #289
    [1, 1, 0, 0], // #290
    [1, 1, 0, 0], // EnqueueMarker
    [1, 1, 0, 0], // EnqueueKernel
    [1, 1, 0, 0], // GetKernelNDrangeSubGroupCount
    [1, 1, 0, 0], // GetKernelNDrangeMaxSubGroupSize
    [1, 1, 0, 0], // GetKernelWorkGroupSize
    [1, 1, 0, 0], // GetKernelPreferredWorkGroupSizeMultiple
    [0, 0, 0, 0], // RetainEvent
    [0, 0, 0, 0], // ReleaseEvent
    [1, 1, 0, 0], // CreateUserEvent
    [1, 1, 0, 0], // IsValidEvent
    [0, 0, 0, 0], // SetUserEventStatus
    [0, 0, 0, 0], // CaptureEventProfilingInfo
    [1, 1, 0, 0], // GetDefaultQueue
    [1, 1, 0, 0], // BuildNDRange
    [1, 1, 2, 1], // ImageSparseSampleImplicitLod
    [1, 1, 2, 1], // ImageSparseSampleExplicitLod
    [1, 1, 3, 1], // ImageSparseSampleDrefImplicitLod
    [1, 1, 3, 1], // ImageSparseSampleDrefExplicitLod
    [1, 1, 2, 1], // ImageSparseSampleProjImplicitLod
    [1, 1, 2, 1], // ImageSparseSampleProjExplicitLod
    [1, 1, 3, 1], // ImageSparseSampleProjDrefImplicitLod
    [1, 1, 3, 1], // ImageSparseSampleProjDrefExplicitLod
    [1, 1, 2, 1], // ImageSparseFetch
    [1, 1, 3, 1], // ImageSparseGather
    [1, 1, 3, 1], // ImageSparseDrefGather
    [1, 1, 1, 0], // ImageSparseTexelsResident
    [0, 0, 0, 0], // NoLine
    [1, 1, 0, 0], // AtomicFlagTestAndSet
    [0, 0, 0, 0], // AtomicFlagClear
    [1, 1, 0, 0], // ImageSparseRead
    [1, 1, 0, 0], // SizeOf
    [1, 1, 0, 0], // TypePipeStorage
    [1, 1, 0, 0], // ConstantPipeStorage
    [1, 1, 0, 0], // CreatePipeFromPipeStorage
    [1, 1, 0, 0], // GetKernelLocalSizeForSubgroupCount
    [1, 1, 0, 0], // GetKernelMaxNumSubgroups
    [1, 1, 0, 0], // TypeNamedBarrier
    [1, 1, 0, 1], // NamedBarrierInitialize
    [0, 0, 2, 1], // MemoryNamedBarrier
    [1, 1, 0, 0], // ModuleProcessed
    [0, 0, 0, 1], // ExecutionModeId
    [0, 0, 0, 1], // DecorateId
    [1, 1, 1, 1], // GroupNonUniformElect
    [1, 1, 1, 1], // GroupNonUniformAll
    [1, 1, 1, 1], // GroupNonUniformAny
    [1, 1, 1, 1], // GroupNonUniformAllEqual
    [1, 1, 1, 1], // GroupNonUniformBroadcast
    [1, 1, 1, 1], // GroupNonUniformBroadcastFirst
    [1, 1, 1, 1], // GroupNonUniformBallot
    [1, 1, 1, 1], // GroupNonUniformInverseBallot
    [1, 1, 1, 1], // GroupNonUniformBallotBitExtract
    [1, 1, 1, 1], // GroupNonUniformBallotBitCount
    [1, 1, 1, 1], // GroupNonUniformBallotFindLSB
    [1, 1, 1, 1], // GroupNonUniformBallotFindMSB
    [1, 1, 1, 1], // GroupNonUniformShuffle
    [1, 1, 1, 1], // GroupNonUniformShuffleXor
    [1, 1, 1, 1], // GroupNonUniformShuffleUp
    [1, 1, 1, 1], // GroupNonUniformShuffleDown
    [1, 1, 1, 1], // GroupNonUniformIAdd
    [1, 1, 1, 1], // GroupNonUniformFAdd
    [1, 1, 1, 1], // GroupNonUniformIMul
    [1, 1, 1, 1], // GroupNonUniformFMul
    [1, 1, 1, 1], // GroupNonUniformSMin
    [1, 1, 1, 1], // GroupNonUniformUMin
    [1, 1, 1, 1], // GroupNonUniformFMin
    [1, 1, 1, 1], // GroupNonUniformSMax
    [1, 1, 1, 1], // GroupNonUniformUMax
    [1, 1, 1, 1], // GroupNonUniformFMax
    [1, 1, 1, 1], // GroupNonUniformBitwiseAnd
    [1, 1, 1, 1], // GroupNonUniformBitwiseOr
    [1, 1, 1, 1], // GroupNonUniformBitwiseXor
    [1, 1, 1, 1], // GroupNonUniformLogicalAnd
    [1, 1, 1, 1], // GroupNonUniformLogicalOr
    [1, 1, 1, 1], // GroupNonUniformLogicalXor
    [1, 1, 1, 1], // GroupNonUniformQuadBroadcast
    [1, 1, 1, 1], // GroupNonUniformQuadSwap
];

fn known_ops(version: u32) -> usize {
    match version {
        0 => OP_MODULE_PROCESSED as usize + 1,
        1 => OP_DATA.len(),
        _ => 0,
    }
}

fn op_data(op: u32, n: usize) -> [u8; 4] {
    if (op as usize) < n { OP_DATA[op as usize] } else { [0; 4] }
}

fn remap(op: u32) -> u32 {
    const SWAPS: [(u32, u32); 13] = [
        (71, 0),   // Decorate <-> Nop
        (61, 1),   // Load <-> Undef
        (62, 2),   // Store <-> SourceContinued
        (65, 3),   // AccessChain <-> Source
        (79, 4),   // VectorShuffle <-> SourceExtension
        (72, 7),   // MemberDecorate <-> String
        (248, 8),  // Label <-> Line
        (59, 9),   // Variable <-> 9
        (133, 10), // FMul <-> Extension
        (129, 11), // FAdd <-> ExtInstImport
        (32, 14),  // TypePointer <-> MemoryModel
        (127, 15), // FNegate <-> EntryPoint
        (u32::MAX, u32::MAX),
    ];
    for (a, b) in SWAPS {
        if op == a {
            return b;
        }
        if op == b {
            return a;
        }
    }
    op
}

fn decode_len(op: u32, len: u32) -> u32 {
    len + 1
        + match op {
            OP_VECTOR_SHUFFLE | OP_VECTOR_SHUFFLE_COMPACT => 4,
            OP_DECORATE => 2,
            OP_LOAD | OP_ACCESS_CHAIN => 3,
            _ => 0,
        }
}

fn zig(u: u32) -> u32 {
    if u & 1 != 0 { (u >> 1) ^ !0 } else { u >> 1 }
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn varint(&mut self) -> u32 {
        let (mut v, mut shift) = (0u32, 0u32);
        while self.i < self.b.len() {
            let x = self.b[self.i];
            self.i += 1;
            if shift < 32 {
                v |= ((x & 127) as u32) << shift;
            }
            shift += 7;
            if x & 128 == 0 {
                break;
            }
        }
        v
    }
    fn u4(&mut self) -> Option<u32> {
        let w = self.b.get(self.i..self.i + 4)?;
        self.i += 4;
        Some(u32::from_le_bytes(w.try_into().unwrap()))
    }
    fn byte(&mut self) -> Option<u8> {
        let x = *self.b.get(self.i)?;
        self.i += 1;
        Some(x)
    }
}

/// Decoded SPIR-V size in bytes, if `data` starts with a valid SMOL-V header.
pub fn decoded_size(data: &[u8]) -> Option<usize> {
    if data.len() < 24 {
        return None;
    }
    let w = |i: usize| u32::from_le_bytes(data[i * 4..i * 4 + 4].try_into().unwrap());
    let ver = w(1) & 0x00FF_FFFF;
    if w(0) != SMOL_MAGIC || !(0x0001_0000..=0x0001_0600).contains(&ver) || (w(1) >> 24) > 1 {
        return None;
    }
    Some(w(5) as usize)
}

/// Decodes one SMOL-V module (the slice may extend past its end; decoding stops at the declared size).
pub fn decode(data: &[u8]) -> Option<Vec<u32>> {
    let size = decoded_size(data)?;
    let mut out: Vec<u32> = Vec::with_capacity(size / 4);
    let mut r = Reader { b: data, i: 4 };
    out.push(SPIRV_MAGIC);
    let v = r.u4()?;
    let smol_version = v >> 24;
    out.push(v & 0x00FF_FFFF);
    for _ in 0..3 {
        out.push(r.u4()?); // generator, bound, schema
    }
    r.i += 4; // decoded size
    let n = known_ops(smol_version);
    let (mut prev_result, mut prev_decorate) = (0u32, 0u32);
    while out.len() * 4 < size && r.i < r.b.len() {
        let val = r.varint();
        let mut len = ((val >> 20) << 4) | ((val >> 4) & 0xF);
        let mut op = ((val >> 4) & 0xFFF0) | (val & 0xF);
        op = remap(op);
        len = decode_len(op, len);
        let was_swizzle = op == OP_VECTOR_SHUFFLE_COMPACT;
        if was_swizzle {
            op = OP_VECTOR_SHUFFLE;
        }
        out.push((len << 16) | op);
        let mut ioffs = 1u32;
        let [has_result, has_type, delta, varrest] = op_data(op, n);
        if has_type != 0 {
            out.push(r.varint());
            ioffs += 1;
        }
        if has_result != 0 {
            let v = prev_result.wrapping_add(zig(r.varint()));
            out.push(v);
            prev_result = v;
            ioffs += 1;
        }
        if op == OP_DECORATE || op == OP_MEMBER_DECORATE {
            let v = prev_decorate.wrapping_add(zig(r.varint()));
            out.push(v);
            prev_decorate = v;
            ioffs += 1;
        }
        if op == OP_MEMBER_DECORATE {
            let count = r.byte()?;
            let (mut prev_index, mut prev_offset) = (0u32, 0u32);
            for m in 0..count {
                let index = r.varint().wrapping_add(prev_index);
                prev_index = index;
                let dec = r.varint();
                let extra = match dec {
                    0 | 2..=5 => Some(0),
                    29..=37 => Some(1),
                    _ => None,
                };
                let mlen = match extra {
                    Some(e) => 4 + e,
                    None => r.varint() + 4,
                };
                if m != 0 {
                    out.push((mlen << 16) | op);
                    out.push(prev_decorate);
                }
                out.push(index);
                out.push(dec);
                if dec == 35 {
                    if mlen != 5 {
                        return None;
                    }
                    let v = r.varint().wrapping_add(prev_offset);
                    out.push(v);
                    prev_offset = v;
                } else {
                    for _ in 4..mlen {
                        out.push(r.varint());
                    }
                }
            }
            continue;
        }
        let mut i = 0;
        while i < delta && ioffs < len {
            let v = zig(r.varint());
            out.push(prev_result.wrapping_sub(v));
            i += 1;
            ioffs += 1;
        }
        if was_swizzle && len <= 9 {
            let s = r.byte()? as u32;
            for (k, shift) in [(5, 6), (6, 4), (7, 2), (8, 0)] {
                if len > k {
                    out.push((s >> shift) & 3);
                }
            }
        } else if varrest != 0 {
            while ioffs < len {
                out.push(r.varint());
                ioffs += 1;
            }
        } else {
            while ioffs < len {
                out.push(r.u4()?);
                ioffs += 1;
            }
        }
    }
    (out.len() * 4 == size).then_some(out)
}
