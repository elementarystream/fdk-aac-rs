// Differential tests of fdk_aac_rs_fdk::matrix_calloc against the C++ reference.
//
// The allocations are compared by their layout: NULL or not, the start of every row (and plane)
// in elements from the start of the contiguous block, and zero initialization.

use fdk_aac_rs_cref::fdk_matrix_calloc as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::matrix_calloc::*;

/// Offset of `row` from `base`, in elements of `T`.
fn offset<T>(base: &[T], row: &[T]) -> i32 {
    ((row.as_ptr() as usize - base.as_ptr() as usize) / std::mem::size_of::<T>()) as i32
}

fn dims(rng: &mut Rng) -> u32 {
    match rng.next_u32() % 5 {
        0 => 0,
        1 => 1,
        _ => rng.next_u32() % 40,
    }
}

#[test]
fn fdk_calloc_matrix_1d_() {
    let mut rng = Rng::new(501);
    for _ in 0..500 {
        let d = dims(&mut rng);
        for variant in 0..4 {
            let v: Vec<i32> = match variant {
                0 => fdk_calloc_matrix_1d(d),
                1 => fdk_calloc_matrix_1d_aligned(d),
                2 => fdk_calloc_matrix_1d_int(d, 0),
                _ => fdk_calloc_matrix_1d_int_aligned(d, 0),
            };
            let e = unsafe { c::cref_mc_1d(d, 4, variant) };
            // calloc(0, ...) and the aligned allocations of 0 bytes are not NULL on the
            // reference platforms.
            assert_eq!(e, 1, "C fdkCallocMatrix1D variant {variant} ({d})");
            assert_eq!(v.len(), d as usize);
            assert!(v.iter().all(|&x| x == 0));
            if variant % 2 == 0 {
                fdk_free_matrix_1d(v);
            } else {
                fdk_free_matrix_1d_aligned(v);
            }
        }
    }
}

#[test]
fn fdk_calloc_matrix_2d_() {
    let mut rng = Rng::new(502);
    for _ in 0..500 {
        let (d1, d2) = (dims(&mut rng), dims(&mut rng));
        for variant in 0..4 {
            let m: Option<Matrix2D<i16>> = match variant {
                0 => fdk_calloc_matrix_2d(d1, d2),
                1 => fdk_calloc_matrix_2d_aligned(d1, d2),
                2 => fdk_calloc_matrix_2d_int(d1, d2, 0),
                _ => fdk_calloc_matrix_2d_int_aligned(d1, d2, 0),
            };
            let mut rows = vec![0i32; d1 as usize];
            let e = unsafe { c::cref_mc_2d(d1, d2, 2, variant, rows.as_mut_ptr()) };
            let what = format!("fdkCallocMatrix2D variant {variant} ({d1}, {d2})");
            match m {
                None => assert_eq!(e, -1, "{what}"),
                Some(m) => {
                    assert_eq!(e, 1, "{what}");
                    assert_eq!((m.dim1(), m.dim2()), (d1 as usize, d2 as usize));
                    assert_eq!(m.as_flat().len(), (d1 * d2) as usize);
                    assert!(m.as_flat().iter().all(|&x| x == 0));
                    for i in 0..d1 as usize {
                        assert_eq!(offset(m.as_flat(), &m[i]), rows[i], "{what}: row {i}");
                        assert_eq!(m[i].len(), d2 as usize);
                    }
                    if variant % 2 == 0 {
                        fdk_free_matrix_2d(Some(m));
                    } else {
                        fdk_free_matrix_2d_aligned(Some(m));
                    }
                }
            }
        }
    }
}

#[test]
fn fdk_calloc_matrix_3d_() {
    let mut rng = Rng::new(503);
    for _ in 0..300 {
        let (d1, d2, d3) = (dims(&mut rng), dims(&mut rng), dims(&mut rng));
        for variant in 0..2 {
            let m: Option<Matrix3D<i32>> = match variant {
                0 => fdk_calloc_matrix_3d(d1, d2, d3),
                _ => fdk_calloc_matrix_3d_int(d1, d2, d3, 0),
            };
            let mut planes = vec![0i32; d1 as usize];
            let mut rows = vec![0i32; (d1 * d2) as usize];
            let e = unsafe { c::cref_mc_3d(d1, d2, d3, 4, variant, planes.as_mut_ptr(), rows.as_mut_ptr()) };
            let what = format!("fdkCallocMatrix3D variant {variant} ({d1}, {d2}, {d3})");
            match m {
                None => assert_eq!(e, -1, "{what}"),
                Some(m) => {
                    assert_eq!(e, 1, "{what}");
                    assert_eq!((m.dim1(), m.dim2(), m.dim3()), (d1 as usize, d2 as usize, d3 as usize));
                    assert!(m.as_flat().iter().all(|&x| x == 0));
                    for i in 0..d1 as usize {
                        // p[i] points to the row pointers of plane i: dim2 apart.
                        assert_eq!((i * d2 as usize) as i32, planes[i], "{what}: plane {i}");
                        assert_eq!(offset(m.as_flat(), m.plane(i)), rows[i * d2 as usize], "{what}: plane {i}");
                        for j in 0..d2 as usize {
                            let r = &m[(i, j)];
                            assert_eq!(r.len(), d3 as usize);
                            assert_eq!(offset(m.as_flat(), r), rows[i * d2 as usize + j], "{what}: row {i} {j}");
                        }
                    }
                    fdk_free_matrix_3d(Some(m));
                }
            }
        }
    }
}
