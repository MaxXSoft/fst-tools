/// Type of packaging method of writer.
pub use crate::capi::fstWriterPackType as WriterPackType;

/// Enum values of type [`WriterPackType`](crate::WriterPackType).
pub mod writer_pack_type {
  pub use crate::capi::fstWriterPackType_FST_WR_PT_FASTLZ as FASTLZ;
  pub use crate::capi::fstWriterPackType_FST_WR_PT_LZ4 as LZ4;
  pub use crate::capi::fstWriterPackType_FST_WR_PT_ZLIB as ZLIB;
}

/// Type of file.
pub use crate::capi::fstFileType as FileType;

/// Enum values of type [`FileType`](crate::FileType).
pub mod file_type {
  pub use crate::capi::fstFileType_FST_FT_MAX as MAX;
  pub use crate::capi::fstFileType_FST_FT_MIN as MIN;
  pub use crate::capi::fstFileType_FST_FT_VERILOG as VERILOG;
  pub use crate::capi::fstFileType_FST_FT_VERILOG_VHDL as VERILOG_VHDL;
  pub use crate::capi::fstFileType_FST_FT_VHDL as VHDL;
}

/// Type of block.
pub use crate::capi::fstBlockType as BlockType;

/// Enum values of type [`BlockType`](crate::BlockType).
pub mod block_type {
  pub use crate::capi::fstBlockType_FST_BL_BLACKOUT as BLACKOUT;
  pub use crate::capi::fstBlockType_FST_BL_GEOM as GEOM;
  pub use crate::capi::fstBlockType_FST_BL_HDR as HDR;
  pub use crate::capi::fstBlockType_FST_BL_HIER as HIER;
  pub use crate::capi::fstBlockType_FST_BL_HIER_LZ4 as HIER_LZ4;
  pub use crate::capi::fstBlockType_FST_BL_HIER_LZ4DUO as HIER_LZ4DUO;
  pub use crate::capi::fstBlockType_FST_BL_SKIP as SKIP;
  pub use crate::capi::fstBlockType_FST_BL_VCDATA as VCDATA;
  pub use crate::capi::fstBlockType_FST_BL_VCDATA_DYN_ALIAS as VCDATA_DYN_ALIAS;
  pub use crate::capi::fstBlockType_FST_BL_VCDATA_DYN_ALIAS2 as VCDATA_DYN_ALIAS2;
  pub use crate::capi::fstBlockType_FST_BL_ZWRAPPER as ZWRAPPER;
}

/// Type of scope.
pub use crate::capi::fstScopeType as ScopeType;

/// Enum values of type [`ScopeType`](crate::ScopeType).
pub mod scope_type {
  pub use crate::capi::fstScopeType_FST_ST_GEN_ATTRBEGIN as GEN_ATTRBEGIN;
  pub use crate::capi::fstScopeType_FST_ST_GEN_ATTREND as GEN_ATTREND;
  pub use crate::capi::fstScopeType_FST_ST_MAX as MAX;
  pub use crate::capi::fstScopeType_FST_ST_MIN as MIN;
  pub use crate::capi::fstScopeType_FST_ST_VCD_BEGIN as VCD_BEGIN;
  pub use crate::capi::fstScopeType_FST_ST_VCD_CLASS as VCD_CLASS;
  pub use crate::capi::fstScopeType_FST_ST_VCD_FORK as VCD_FORK;
  pub use crate::capi::fstScopeType_FST_ST_VCD_FUNCTION as VCD_FUNCTION;
  pub use crate::capi::fstScopeType_FST_ST_VCD_GENERATE as VCD_GENERATE;
  pub use crate::capi::fstScopeType_FST_ST_VCD_INTERFACE as VCD_INTERFACE;
  pub use crate::capi::fstScopeType_FST_ST_VCD_MODULE as VCD_MODULE;
  pub use crate::capi::fstScopeType_FST_ST_VCD_PACKAGE as VCD_PACKAGE;
  pub use crate::capi::fstScopeType_FST_ST_VCD_PROGRAM as VCD_PROGRAM;
  pub use crate::capi::fstScopeType_FST_ST_VCD_SCOPE as VCD_SCOPE;
  pub use crate::capi::fstScopeType_FST_ST_VCD_STRUCT as VCD_STRUCT;
  pub use crate::capi::fstScopeType_FST_ST_VCD_TASK as VCD_TASK;
  pub use crate::capi::fstScopeType_FST_ST_VCD_UNION as VCD_UNION;
  pub use crate::capi::fstScopeType_FST_ST_VCD_UPSCOPE as VCD_UPSCOPE;
  pub use crate::capi::fstScopeType_FST_ST_VHDL_ARCHITECTURE as VHDL_ARCHITECTURE;
  pub use crate::capi::fstScopeType_FST_ST_VHDL_BLOCK as VHDL_BLOCK;
  pub use crate::capi::fstScopeType_FST_ST_VHDL_FOR_GENERATE as VHDL_FOR_GENERATE;
  pub use crate::capi::fstScopeType_FST_ST_VHDL_FUNCTION as VHDL_FUNCTION;
  pub use crate::capi::fstScopeType_FST_ST_VHDL_GENERATE as VHDL_GENERATE;
  pub use crate::capi::fstScopeType_FST_ST_VHDL_IF_GENERATE as VHDL_IF_GENERATE;
  pub use crate::capi::fstScopeType_FST_ST_VHDL_PACKAGE as VHDL_PACKAGE;
  pub use crate::capi::fstScopeType_FST_ST_VHDL_PROCEDURE as VHDL_PROCEDURE;
  pub use crate::capi::fstScopeType_FST_ST_VHDL_PROCESS as VHDL_PROCESS;
  pub use crate::capi::fstScopeType_FST_ST_VHDL_RECORD as VHDL_RECORD;
}

/// Type of variable.
pub use crate::capi::fstVarType as VarType;

/// Enum values of type [`VarType`](crate::VarType).
pub mod var_type {
  pub use crate::capi::fstVarType_FST_VT_GEN_STRING as GEN_STRING;
  pub use crate::capi::fstVarType_FST_VT_MAX as MAX;
  pub use crate::capi::fstVarType_FST_VT_MIN as MIN;
  pub use crate::capi::fstVarType_FST_VT_SV_BIT as SV_BIT;
  pub use crate::capi::fstVarType_FST_VT_SV_BYTE as SV_BYTE;
  pub use crate::capi::fstVarType_FST_VT_SV_ENUM as SV_ENUM;
  pub use crate::capi::fstVarType_FST_VT_SV_INT as SV_INT;
  pub use crate::capi::fstVarType_FST_VT_SV_LOGIC as SV_LOGIC;
  pub use crate::capi::fstVarType_FST_VT_SV_LONGINT as SV_LONGINT;
  pub use crate::capi::fstVarType_FST_VT_SV_SHORTINT as SV_SHORTINT;
  pub use crate::capi::fstVarType_FST_VT_SV_SHORTREAL as SV_SHORTREAL;
  pub use crate::capi::fstVarType_FST_VT_VCD_EVENT as VCD_EVENT;
  pub use crate::capi::fstVarType_FST_VT_VCD_INTEGER as VCD_INTEGER;
  pub use crate::capi::fstVarType_FST_VT_VCD_PARAMETER as VCD_PARAMETER;
  pub use crate::capi::fstVarType_FST_VT_VCD_PORT as VCD_PORT;
  pub use crate::capi::fstVarType_FST_VT_VCD_REAL as VCD_REAL;
  pub use crate::capi::fstVarType_FST_VT_VCD_REALTIME as VCD_REALTIME;
  pub use crate::capi::fstVarType_FST_VT_VCD_REAL_PARAMETER as VCD_REAL_PARAMETER;
  pub use crate::capi::fstVarType_FST_VT_VCD_REG as VCD_REG;
  pub use crate::capi::fstVarType_FST_VT_VCD_SPARRAY as VCD_SPARRAY;
  pub use crate::capi::fstVarType_FST_VT_VCD_SUPPLY0 as VCD_SUPPLY0;
  pub use crate::capi::fstVarType_FST_VT_VCD_SUPPLY1 as VCD_SUPPLY1;
  pub use crate::capi::fstVarType_FST_VT_VCD_TIME as VCD_TIME;
  pub use crate::capi::fstVarType_FST_VT_VCD_TRI as VCD_TRI;
  pub use crate::capi::fstVarType_FST_VT_VCD_TRI0 as VCD_TRI0;
  pub use crate::capi::fstVarType_FST_VT_VCD_TRI1 as VCD_TRI1;
  pub use crate::capi::fstVarType_FST_VT_VCD_TRIAND as VCD_TRIAND;
  pub use crate::capi::fstVarType_FST_VT_VCD_TRIOR as VCD_TRIOR;
  pub use crate::capi::fstVarType_FST_VT_VCD_TRIREG as VCD_TRIREG;
  pub use crate::capi::fstVarType_FST_VT_VCD_WAND as VCD_WAND;
  pub use crate::capi::fstVarType_FST_VT_VCD_WIRE as VCD_WIRE;
  pub use crate::capi::fstVarType_FST_VT_VCD_WOR as VCD_WOR;
}

/// Type of variable direction.
pub use crate::capi::fstVarDir as VarDir;

/// Enum values of type [`VarDir`](crate::VarDir).
pub mod var_dir {
  pub use crate::capi::fstVarDir_FST_VD_BUFFER as BUFFER;
  pub use crate::capi::fstVarDir_FST_VD_IMPLICIT as IMPLICIT;
  pub use crate::capi::fstVarDir_FST_VD_INOUT as INOUT;
  pub use crate::capi::fstVarDir_FST_VD_INPUT as INPUT;
  pub use crate::capi::fstVarDir_FST_VD_LINKAGE as LINKAGE;
  pub use crate::capi::fstVarDir_FST_VD_MAX as MAX;
  pub use crate::capi::fstVarDir_FST_VD_MIN as MIN;
  pub use crate::capi::fstVarDir_FST_VD_OUTPUT as OUTPUT;
}

/// Type of attribute.
pub use crate::capi::fstAttrType as AttrType;

/// Enum values of type [`AttrType`](crate::AttrType).
pub mod attr_type {
  pub use crate::capi::fstAttrType_FST_AT_ARRAY as ARRAY;
  pub use crate::capi::fstAttrType_FST_AT_ENUM as ENUM;
  pub use crate::capi::fstAttrType_FST_AT_MAX as MAX;
  pub use crate::capi::fstAttrType_FST_AT_MIN as MIN;
  pub use crate::capi::fstAttrType_FST_AT_MISC as MISC;
  pub use crate::capi::fstAttrType_FST_AT_PACK as PACK;
}

/// Subtype of the attribute of type `MISC`.
pub use crate::capi::fstMiscType as MiscType;

/// Enum values of type [`MiscType`](crate::MiscType).
pub mod misc_type {
  pub use crate::capi::fstMiscType_FST_MT_COMMENT as COMMENT;
  pub use crate::capi::fstMiscType_FST_MT_ENUMTABLE as ENUMTABLE;
  pub use crate::capi::fstMiscType_FST_MT_ENVVAR as ENVVAR;
  pub use crate::capi::fstMiscType_FST_MT_MAX as MAX;
  pub use crate::capi::fstMiscType_FST_MT_MIN as MIN;
  pub use crate::capi::fstMiscType_FST_MT_PATHNAME as PATHNAME;
  pub use crate::capi::fstMiscType_FST_MT_SOURCEISTEM as SOURCEISTEM;
  pub use crate::capi::fstMiscType_FST_MT_SOURCESTEM as SOURCESTEM;
  pub use crate::capi::fstMiscType_FST_MT_SUPVAR as SUPVAR;
  pub use crate::capi::fstMiscType_FST_MT_UNKNOWN as UNKNOWN;
  pub use crate::capi::fstMiscType_FST_MT_VALUELIST as VALUELIST;
}

/// Subtype of the attribute of type `ARRAY`.
pub use crate::capi::fstArrayType as ArrayType;

/// Enum values of type [`ArrayType`](crate::ArrayType).
pub mod array_type {
  pub use crate::capi::fstArrayType_FST_AR_MAX as MAX;
  pub use crate::capi::fstArrayType_FST_AR_MIN as MIN;
  pub use crate::capi::fstArrayType_FST_AR_NONE as NONE;
  pub use crate::capi::fstArrayType_FST_AR_PACKED as PACKED;
  pub use crate::capi::fstArrayType_FST_AR_SPARSE as SPARSE;
  pub use crate::capi::fstArrayType_FST_AR_UNPACKED as UNPACKED;
}

/// Subtype of the attribute of type `ENUM`.
pub use crate::capi::fstEnumValueType as EnumValueType;

/// Enum values of type [`EnumValueType`](crate::EnumValueType).
pub mod enum_value_type {
  pub use crate::capi::fstEnumValueType_FST_EV_MAX as MAX;
  pub use crate::capi::fstEnumValueType_FST_EV_REG as REG;
  pub use crate::capi::fstEnumValueType_FST_EV_SV_BIT as SV_BIT;
  pub use crate::capi::fstEnumValueType_FST_EV_SV_BYTE as SV_BYTE;
  pub use crate::capi::fstEnumValueType_FST_EV_SV_INT as SV_INT;
  pub use crate::capi::fstEnumValueType_FST_EV_SV_INTEGER as SV_INTEGER;
  pub use crate::capi::fstEnumValueType_FST_EV_SV_LOGIC as SV_LOGIC;
  pub use crate::capi::fstEnumValueType_FST_EV_SV_LONGINT as SV_LONGINT;
  pub use crate::capi::fstEnumValueType_FST_EV_SV_SHORTINT as SV_SHORTINT;
  pub use crate::capi::fstEnumValueType_FST_EV_SV_UNSIGNED_BIT as SV_UNSIGNED_BIT;
  pub use crate::capi::fstEnumValueType_FST_EV_SV_UNSIGNED_BYTE as SV_UNSIGNED_BYTE;
  pub use crate::capi::fstEnumValueType_FST_EV_SV_UNSIGNED_INT as SV_UNSIGNED_INT;
  pub use crate::capi::fstEnumValueType_FST_EV_SV_UNSIGNED_INTEGER as SV_UNSIGNED_INTEGER;
  pub use crate::capi::fstEnumValueType_FST_EV_SV_UNSIGNED_LOGIC as SV_UNSIGNED_LOGIC;
  pub use crate::capi::fstEnumValueType_FST_EV_SV_UNSIGNED_LONGINT as SV_UNSIGNED_LONGINT;
  pub use crate::capi::fstEnumValueType_FST_EV_SV_UNSIGNED_SHORTINT as SV_UNSIGNED_SHORTINT;
  pub use crate::capi::fstEnumValueType_FST_EV_TIME as TIME;
}

/// Subtype of the attribute of type `PACK`.
pub use crate::capi::fstPackType as PackType;

/// Enum values of type [`PackType`](crate::PackType).
pub mod pack_type {
  pub use crate::capi::fstPackType_FST_PT_MAX as MAX;
  pub use crate::capi::fstPackType_FST_PT_NONE as NONE;
  pub use crate::capi::fstPackType_FST_PT_PACKED as PACKED;
  pub use crate::capi::fstPackType_FST_PT_TAGGED_PACKED as TAGGED_PACKED;
  pub use crate::capi::fstPackType_FST_PT_UNPACKED as UNPACKED;
}

/// Type of supplemental variable.
pub use crate::capi::fstSupplementalVarType as SupplementalVarType;

/// Enum values of type [`SupplementalVarType`](crate::SupplementalVarType).
pub mod supplemental_var_type {
  pub use crate::capi::fstSupplementalVarType_FST_SVT_MAX as MAX;
  pub use crate::capi::fstSupplementalVarType_FST_SVT_MIN as MIN;
  pub use crate::capi::fstSupplementalVarType_FST_SVT_NONE as NONE;
  pub use crate::capi::fstSupplementalVarType_FST_SVT_VHDL_CONSTANT as VHDL_CONSTANT;
  pub use crate::capi::fstSupplementalVarType_FST_SVT_VHDL_FILE as VHDL_FILE;
  pub use crate::capi::fstSupplementalVarType_FST_SVT_VHDL_MEMORY as VHDL_MEMORY;
  pub use crate::capi::fstSupplementalVarType_FST_SVT_VHDL_SIGNAL as VHDL_SIGNAL;
  pub use crate::capi::fstSupplementalVarType_FST_SVT_VHDL_VARIABLE as VHDL_VARIABLE;
}

/// Type of supplemental data.
pub use crate::capi::fstSupplementalDataType as SupplementalDataType;

/// Enum values of type [`SupplementalDataType`](crate::SupplementalDataType).
pub mod supplemental_data_type {
  pub use crate::capi::fstSupplementalDataType_FST_SDT_ABS_MAX as ABS_MAX;
  pub use crate::capi::fstSupplementalDataType_FST_SDT_MAX as MAX;
  pub use crate::capi::fstSupplementalDataType_FST_SDT_MIN as MIN;
  pub use crate::capi::fstSupplementalDataType_FST_SDT_NONE as NONE;
  pub use crate::capi::fstSupplementalDataType_FST_SDT_SVT_SHIFT_COUNT as SVT_SHIFT_COUNT;
  pub use crate::capi::fstSupplementalDataType_FST_SDT_VHDL_BIT as VHDL_BIT;
  pub use crate::capi::fstSupplementalDataType_FST_SDT_VHDL_BIT_VECTOR as VHDL_BIT_VECTOR;
  pub use crate::capi::fstSupplementalDataType_FST_SDT_VHDL_BOOLEAN as VHDL_BOOLEAN;
  pub use crate::capi::fstSupplementalDataType_FST_SDT_VHDL_CHARACTER as VHDL_CHARACTER;
  pub use crate::capi::fstSupplementalDataType_FST_SDT_VHDL_INTEGER as VHDL_INTEGER;
  pub use crate::capi::fstSupplementalDataType_FST_SDT_VHDL_NATURAL as VHDL_NATURAL;
  pub use crate::capi::fstSupplementalDataType_FST_SDT_VHDL_POSITIVE as VHDL_POSITIVE;
  pub use crate::capi::fstSupplementalDataType_FST_SDT_VHDL_REAL as VHDL_REAL;
  pub use crate::capi::fstSupplementalDataType_FST_SDT_VHDL_SIGNED as VHDL_SIGNED;
  pub use crate::capi::fstSupplementalDataType_FST_SDT_VHDL_STD_LOGIC as VHDL_STD_LOGIC;
  pub use crate::capi::fstSupplementalDataType_FST_SDT_VHDL_STD_LOGIC_VECTOR as VHDL_STD_LOGIC_VECTOR;
  pub use crate::capi::fstSupplementalDataType_FST_SDT_VHDL_STD_ULOGIC as VHDL_STD_ULOGIC;
  pub use crate::capi::fstSupplementalDataType_FST_SDT_VHDL_STD_ULOGIC_VECTOR as VHDL_STD_ULOGIC_VECTOR;
  pub use crate::capi::fstSupplementalDataType_FST_SDT_VHDL_STRING as VHDL_STRING;
  pub use crate::capi::fstSupplementalDataType_FST_SDT_VHDL_TIME as VHDL_TIME;
  pub use crate::capi::fstSupplementalDataType_FST_SDT_VHDL_UNSIGNED as VHDL_UNSIGNED;
}
