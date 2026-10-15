// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/huff_nodes.h, libFDK/src/huff_nodes.cpp.

//! Port of `huff_nodes.h`: the MPEG-D SAC/USAC/SAOC Huffman part 0 tables.
//!
//! A node table is an array of `[SHORT; 2]` nodes, indexed by the node and the next bit: a
//! positive entry is the index of the next node, an entry `<= 0` ends the decoding (the value is
//! `-(entry + 1)`, 0 marks an escape in the 2D tables). The node tables are generated
//! (`tables`); the pointer tables (`FDK_huffICCNodes`, `FDK_huffCLDNodes`, `huffOLDNodes`)
//! are written here with references to them.
//!
//! The node structs are `#[repr(C)]`: their memory is the `SHORT` array of the C++ struct.
//!
//! Every positive entry of every node table is smaller than the length of its table (checked by
//! the differential test), so decoding never leaves a table, whatever the bits.

mod tables;
pub use tables::*;

/// `HUFF_RES_NODES`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct HuffResNodes {
    pub node_tab: [[i16; 2]; 39],
}

// 1D nodes

/// `HUFF_CLD_NOD_1D`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct HuffCldNod1d {
    pub node_tab: [[i16; 2]; 30],
}

/// `HUFF_ICC_NOD_1D`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct HuffIccNod1d {
    pub node_tab: [[i16; 2]; 7],
}

/// `HUFF_CPC_NOD_1D`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct HuffCpcNod1d {
    pub node_tab: [[i16; 2]; 50],
}

/// `HUFF_OLD_NOD_1D`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct HuffOldNod1d {
    pub node_tab: [[i16; 2]; 15],
}

/// `HUFF_NRG_NOD_1D`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct HuffNrgNod1d {
    pub node_tab: [[i16; 2]; 63],
}

// 2D nodes

/// `HUFF_CLD_NOD_2D`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct HuffCldNod2d {
    pub lav3: [[i16; 2]; 15],
    pub lav5: [[i16; 2]; 35],
    pub lav7: [[i16; 2]; 63],
    pub lav9: [[i16; 2]; 99],
}

/// `HUFF_ICC_NOD_2D`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct HuffIccNod2d {
    pub lav1: [[i16; 2]; 3],
    pub lav3: [[i16; 2]; 15],
    pub lav5: [[i16; 2]; 35],
    pub lav7: [[i16; 2]; 63],
}

/// `HUFF_OLD_NOD_2D`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct HuffOldNod2d {
    pub lav3: [[i16; 2]; 15],
    pub lav6: [[i16; 2]; 48],
    pub lav9: [[i16; 2]; 99],
    pub lav12: [[i16; 2]; 168],
}

/// `HUFF_NRG_NOD_2D_df`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct HuffNrgNod2dDf {
    pub lav3: [[i16; 2]; 15],
    pub lav5: [[i16; 2]; 35],
    pub lav7: [[i16; 2]; 63],
    pub lav9: [[i16; 2]; 99],
}

/// `HUFF_NRG_NOD_2D_dt`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct HuffNrgNod2dDt {
    pub lav3: [[i16; 2]; 15],
    pub lav6: [[i16; 2]; 48],
    pub lav9: [[i16; 2]; 99],
    pub lav12: [[i16; 2]; 168],
}

/// `HUFF_NRG_NOD_2D`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct HuffNrgNod2d {
    pub df: [HuffNrgNod2dDf; 2],
    pub dt: [HuffNrgNod2dDt; 2],
    pub dp: [HuffNrgNod2dDf; 2],
}

// Complete bs parameter nodes

/// `HUFF_CLD_NODES`
#[derive(Clone, Copy, Debug)]
pub struct HuffCldNodes {
    pub h1d: [&'static HuffCldNod1d; 3],
    pub h2d: [[&'static HuffCldNod2d; 2]; 3],
}

/// `HUFF_ICC_NODES`
#[derive(Clone, Copy, Debug)]
pub struct HuffIccNodes {
    pub h1d: [&'static HuffIccNod1d; 3],
    pub h2d: [[&'static HuffIccNod2d; 2]; 3],
}

/// `HUFF_OLD_NODES`
#[derive(Clone, Copy, Debug)]
pub struct HuffOldNodes {
    pub h1d: [&'static HuffOldNod1d; 3],
    pub h2d: [[&'static HuffOldNod2d; 2]; 3],
}

/// `HUFF_NRG_NODES`
#[derive(Clone, Copy, Debug)]
pub struct HuffNrgNodes {
    pub h1d: [&'static HuffNrgNod1d; 3],
    pub h2d: &'static HuffNrgNod2d,
}

// parameter instance

/// `HUFF_PT0_NODES`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct HuffPt0Nodes {
    pub cld: [[i16; 2]; 30],
    pub icc: [[i16; 2]; 7],
    pub ipd: [[i16; 2]; 7],
    pub old: [[i16; 2]; 15],
    pub nrg: [[i16; 2]; 63],
}

/// `HUFF_LAV_NODES`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct HuffLavNodes {
    pub node_tab: [[i16; 2]; 3],
}

// USAC specific

/// `HUFF_IPD_NOD_1D`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct HuffIpdNod1d {
    pub node_tab: [[i16; 2]; 7],
}

/// `HUFF_IPD_NOD_2D`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct HuffIpdNod2d {
    pub lav1: [[i16; 2]; 3],
    pub lav3: [[i16; 2]; 15],
    pub lav5: [[i16; 2]; 35],
    pub lav7: [[i16; 2]; 63],
}

/// `HUFF_IPD_NODES`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct HuffIpdNodes {
    pub h1d: [HuffIpdNod1d; 3],
    pub h2d: [[HuffIpdNod2d; 2]; 3],
}

/// `FDK_huffICCNodes`
pub static FDK_HUFF_ICC_NODES: HuffIccNodes = HuffIccNodes {
    h1d: [&FDK_HUFF_ICC_NODES_H1D_0, &FDK_HUFF_ICC_NODES_H1D_0, &FDK_HUFF_ICC_NODES_H1D_0],
    h2d: [
        [&FDK_HUFF_ICC_NODES_H2D_0_0, &FDK_HUFF_ICC_NODES_H2D_0_1],
        [&FDK_HUFF_ICC_NODES_H2D_1_0, &FDK_HUFF_ICC_NODES_H2D_1_1],
        [&FDK_HUFF_ICC_NODES_H2D_0_1, &FDK_HUFF_ICC_NODES_H2D_0_1],
    ],
};

/// `FDK_huffCLDNodes`
pub static FDK_HUFF_CLD_NODES: HuffCldNodes = HuffCldNodes {
    h1d: [&FDK_HUFF_CLD_NODES_H1D_0, &FDK_HUFF_CLD_NODES_H1D_1, &FDK_HUFF_CLD_NODES_H1D_1],
    h2d: [
        [&FDK_HUFF_CLD_NODES_H2_0_0, &FDK_HUFF_CLD_NODES_H2_0_1],
        [&FDK_HUFF_CLD_NODES_H2_1_0, &FDK_HUFF_CLD_NODES_H2_1_1],
        [&FDK_HUFF_CLD_NODES_H2_0_1, &FDK_HUFF_CLD_NODES_H2_0_1],
    ],
};

/// `huffOLDNodes`
pub static HUFF_OLD_NODES: HuffOldNodes = HuffOldNodes {
    h1d: [&HUFF_OLD_NODES_H1D_0, &HUFF_OLD_NODES_H1D_1, &HUFF_OLD_NODES_H1D_1],
    h2d: [
        [&HUFF_OLD_NODES_H2D_0_0, &HUFF_OLD_NODES_H2D_0_1],
        [&HUFF_OLD_NODES_H2D_1_0, &HUFF_OLD_NODES_H2D_1_1],
        [&HUFF_OLD_NODES_H2D_0_1, &HUFF_OLD_NODES_H2D_0_1],
    ],
};
