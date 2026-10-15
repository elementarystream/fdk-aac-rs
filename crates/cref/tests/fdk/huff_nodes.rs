// Differential tests of fdk_aac_rs_fdk::huff_nodes against the C++ reference.
//
// Every node table is compared as memory with the table of the reference (the file-static ones
// from a copy of huff_nodes.cpp compiled into the shim), the pointer tables entry by entry by the
// content of the referenced tables. Every node of every table must stay within its table.

use fdk_aac_rs_cref::fdk_huff_nodes as c;
use fdk_aac_rs_fdk::huff_nodes::*;
use std::ffi::CString;

/// The memory of a node struct (`#[repr(C)]`, only `i16`).
fn shorts<T>(t: &T) -> &[i16] {
    // SAFETY: the node structs consist of i16 arrays.
    unsafe { std::slice::from_raw_parts(t as *const T as *const i16, std::mem::size_of::<T>() / 2) }
}

fn c_table(name: &str) -> &'static [i16] {
    let cname = CString::new(name).unwrap();
    let mut n = 0;
    let p = unsafe { c::cref_huff_nodes_table(cname.as_ptr(), &mut n) };
    assert!(!p.is_null(), "{name} not found in the shim");
    unsafe { std::slice::from_raw_parts(p, n as usize) }
}

fn c_mem(p: *const i16, n: usize) -> &'static [i16] {
    unsafe { std::slice::from_raw_parts(p, n) }
}

#[test]
fn tables_match() {
    macro_rules! check {
        ($($c:literal => $r:ident),* $(,)?) => {
            $(assert_eq!(shorts(&$r), c_table($c), $c);)*
        };
    }
    check!(
        "FDK_huffPart0Nodes" => FDK_HUFF_PART0_NODES,
        "FDK_huffLavIdxNodes" => FDK_HUFF_LAV_IDX_NODES,
        "FDK_huffICCNodes_h1D_0" => FDK_HUFF_ICC_NODES_H1D_0,
        "FDK_huffICCNodes_h2D_0_0" => FDK_HUFF_ICC_NODES_H2D_0_0,
        "FDK_huffICCNodes_h2D_0_1" => FDK_HUFF_ICC_NODES_H2D_0_1,
        "FDK_huffICCNodes_h2D_1_0" => FDK_HUFF_ICC_NODES_H2D_1_0,
        "FDK_huffICCNodes_h2D_1_1" => FDK_HUFF_ICC_NODES_H2D_1_1,
        "FDK_huffCLDNodes_h1D_0" => FDK_HUFF_CLD_NODES_H1D_0,
        "FDK_huffCLDNodes_h1D_1" => FDK_HUFF_CLD_NODES_H1D_1,
        "FDK_huffCLDNodes_h2_0_0" => FDK_HUFF_CLD_NODES_H2_0_0,
        "FDK_huffCLDNodes_h2_0_1" => FDK_HUFF_CLD_NODES_H2_0_1,
        "FDK_huffCLDNodes_h2_1_0" => FDK_HUFF_CLD_NODES_H2_1_0,
        "FDK_huffCLDNodes_h2_1_1" => FDK_HUFF_CLD_NODES_H2_1_1,
        "FDK_huffReshapeNodes" => FDK_HUFF_RESHAPE_NODES,
        "FDK_huffIPDNodes" => FDK_HUFF_IPD_NODES,
        "huffOLDNodes_h1D_0" => HUFF_OLD_NODES_H1D_0,
        "huffOLDNodes_h1D_1" => HUFF_OLD_NODES_H1D_1,
        "huffOLDNodes_h2D_0_0" => HUFF_OLD_NODES_H2D_0_0,
        "huffOLDNodes_h2D_0_1" => HUFF_OLD_NODES_H2D_0_1,
        "huffOLDNodes_h2D_1_0" => HUFF_OLD_NODES_H2D_1_0,
        "huffOLDNodes_h2D_1_1" => HUFF_OLD_NODES_H2D_1_1,
    );
}

#[test]
fn library_tables_match() {
    let g = |id| unsafe { c::cref_huff_nodes_global(id) };
    let p0 = shorts(&FDK_HUFF_PART0_NODES);
    assert_eq!(p0, c_mem(g(0), p0.len()));
    let lav = shorts(&FDK_HUFF_LAV_IDX_NODES);
    assert_eq!(lav, c_mem(g(1), lav.len()));
    let res = shorts(&FDK_HUFF_RESHAPE_NODES);
    assert_eq!(res, c_mem(g(2), res.len()));
    let ipd = shorts(&FDK_HUFF_IPD_NODES);
    assert_eq!(ipd, c_mem(g(3), ipd.len()));
}

#[test]
fn pointer_tables_match() {
    let p = |kind, dim, i, j| unsafe { c::cref_huff_nodes_pointer(kind, dim, i, j) };
    for i in 0..3 {
        let t = shorts(FDK_HUFF_ICC_NODES.h1d[i]);
        assert_eq!(t, c_mem(p(0, 1, i as i32, 0), t.len()), "ICC h1D[{i}]");
        let t = shorts(FDK_HUFF_CLD_NODES.h1d[i]);
        assert_eq!(t, c_mem(p(1, 1, i as i32, 0), t.len()), "CLD h1D[{i}]");
        let t = shorts(HUFF_OLD_NODES.h1d[i]);
        assert_eq!(t, c_mem(p(2, 1, i as i32, 0), t.len()), "OLD h1D[{i}]");
        for j in 0..2 {
            let t = shorts(FDK_HUFF_ICC_NODES.h2d[i][j]);
            assert_eq!(t, c_mem(p(0, 2, i as i32, j as i32), t.len()), "ICC h2D[{i}][{j}]");
            let t = shorts(FDK_HUFF_CLD_NODES.h2d[i][j]);
            assert_eq!(t, c_mem(p(1, 2, i as i32, j as i32), t.len()), "CLD h2D[{i}][{j}]");
            let t = shorts(HUFF_OLD_NODES.h2d[i][j]);
            assert_eq!(t, c_mem(p(2, 2, i as i32, j as i32), t.len()), "OLD h2D[{i}][{j}]");
        }
    }
}

/// Every positive node index is within its table: decoding never leaves a table.
#[test]
fn nodes_stay_in_tables() {
    let mut tables: Vec<(&str, &[[i16; 2]])> = vec![
        ("part0 cld", &FDK_HUFF_PART0_NODES.cld),
        ("part0 icc", &FDK_HUFF_PART0_NODES.icc),
        ("part0 ipd", &FDK_HUFF_PART0_NODES.ipd),
        ("part0 old", &FDK_HUFF_PART0_NODES.old),
        ("part0 nrg", &FDK_HUFF_PART0_NODES.nrg),
        ("lav", &FDK_HUFF_LAV_IDX_NODES.node_tab),
        ("reshape", &FDK_HUFF_RESHAPE_NODES.node_tab),
    ];
    for i in 0..3 {
        tables.push(("icc 1d", &FDK_HUFF_ICC_NODES.h1d[i].node_tab));
        tables.push(("cld 1d", &FDK_HUFF_CLD_NODES.h1d[i].node_tab));
        tables.push(("old 1d", &HUFF_OLD_NODES.h1d[i].node_tab));
        tables.push(("ipd 1d", &FDK_HUFF_IPD_NODES.h1d[i].node_tab));
        for j in 0..2 {
            let t = FDK_HUFF_ICC_NODES.h2d[i][j];
            tables.extend([("icc lav1", &t.lav1[..]), ("icc lav3", &t.lav3), ("icc lav5", &t.lav5)]);
            tables.push(("icc lav7", &t.lav7));
            let t = FDK_HUFF_CLD_NODES.h2d[i][j];
            tables.extend([("cld lav3", &t.lav3[..]), ("cld lav5", &t.lav5), ("cld lav7", &t.lav7)]);
            tables.push(("cld lav9", &t.lav9));
            let t = HUFF_OLD_NODES.h2d[i][j];
            tables.extend([("old lav3", &t.lav3[..]), ("old lav6", &t.lav6), ("old lav9", &t.lav9)]);
            tables.push(("old lav12", &t.lav12));
            let t = &FDK_HUFF_IPD_NODES.h2d[i][j];
            tables.extend([("ipd lav1", &t.lav1[..]), ("ipd lav3", &t.lav3), ("ipd lav5", &t.lav5)]);
            tables.push(("ipd lav7", &t.lav7));
        }
    }
    for (name, t) in tables {
        for node in t {
            for &v in node {
                assert!((v as i32) < t.len() as i32, "{name}: node {v} beyond {} entries", t.len());
            }
        }
    }
}
