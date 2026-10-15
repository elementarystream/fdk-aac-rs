// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/FDK_matrixCalloc.h, libFDK/src/FDK_matrixCalloc.cpp.

//! Port of `FDK_matrixCalloc.h`: matrix memory allocation.
//!
//! The allocations become owned values (PORTING.md, "Memory"):
//! `fdkCallocMatrix1D()` returns a zero-initialized `Vec`, `fdkCallocMatrix2D()` a [`Matrix2D`]
//! and `fdkCallocMatrix3D()` a [`Matrix3D`]. As in C, all elements of a matrix are one
//! contiguous block in row-major order (C code relies on `p[0]` pointing to the whole block, for
//! example for `FDKmemclear`), see [`Matrix2D::as_flat`]. Freeing is dropping the value; the
//! `fdkFree*` functions exist for the mechanical translation.
//!
//! Differences to C: the memory section (`MEMORY_SECTION`) and the alignment have no effect; an
//! allocation failure aborts instead of returning `NULL`; the element counts are computed without
//! the `UINT` wrap-around of `dim1 * dim2 * size` in C (which only matters for sizes that do not
//! fit into memory).

use core::ops::{Index, IndexMut};

pub use crate::generic_stds::MemorySection;

/// `fdkCallocMatrix1D(dim1, size)`: a zero-initialized vector of `dim1` elements.
pub fn fdk_calloc_matrix_1d<T: Clone + Default>(dim1: u32) -> Vec<T> {
    vec![T::default(); dim1 as usize]
}

/// `fdkCallocMatrix1D_aligned(dim1, size)`
pub fn fdk_calloc_matrix_1d_aligned<T: Clone + Default>(dim1: u32) -> Vec<T> {
    fdk_calloc_matrix_1d(dim1)
}

/// `fdkCallocMatrix1D_int(dim, size, s)`
pub fn fdk_calloc_matrix_1d_int<T: Clone + Default>(dim: u32, _s: MemorySection) -> Vec<T> {
    fdk_calloc_matrix_1d(dim)
}

/// `fdkCallocMatrix1D_int_aligned(dim, size, s)`
pub fn fdk_calloc_matrix_1d_int_aligned<T: Clone + Default>(dim: u32, _s: MemorySection) -> Vec<T> {
    fdk_calloc_matrix_1d(dim)
}

/// `fdkFreeMatrix1D(p)`
pub fn fdk_free_matrix_1d<T>(_p: Vec<T>) {}

/// `fdkFreeMatrix1D_aligned(p)`
pub fn fdk_free_matrix_1d_aligned<T>(_p: Vec<T>) {}

/// A zero-initialized `dim1` x `dim2` matrix with contiguous row-major storage
/// (`fdkCallocMatrix2D`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Matrix2D<T> {
    data: Vec<T>,
    dim1: usize,
    dim2: usize,
}

impl<T: Clone + Default> Matrix2D<T> {
    /// A zero-initialized `dim1` x `dim2` matrix.
    ///
    /// # Panics
    /// If the number of elements overflows `usize` (no such allocation could succeed).
    pub fn new(dim1: usize, dim2: usize) -> Self {
        let n = dim1.checked_mul(dim2).expect("matrix size overflows usize");
        Matrix2D { data: vec![T::default(); n], dim1, dim2 }
    }
}

impl<T> Matrix2D<T> {
    /// The number of rows.
    pub fn dim1(&self) -> usize {
        self.dim1
    }

    /// The number of columns.
    pub fn dim2(&self) -> usize {
        self.dim2
    }

    /// All elements in row-major order (`p[0][0 .. dim1 * dim2]` in C).
    pub fn as_flat(&self) -> &[T] {
        &self.data
    }

    /// All elements in row-major order (`p[0][0 .. dim1 * dim2]` in C).
    pub fn as_flat_mut(&mut self) -> &mut [T] {
        &mut self.data
    }

    /// The rows.
    pub fn rows(&self) -> core::slice::ChunksExact<'_, T> {
        self.data.chunks_exact(self.dim2.max(1))
    }

    /// The rows, mutable (gives access to several rows at once).
    pub fn rows_mut(&mut self) -> core::slice::ChunksExactMut<'_, T> {
        self.data.chunks_exact_mut(self.dim2.max(1))
    }
}

impl<T> Index<usize> for Matrix2D<T> {
    type Output = [T];

    /// Row `row` (`p[row]`).
    fn index(&self, row: usize) -> &[T] {
        assert!(row < self.dim1, "Matrix2D row index out of range");
        &self.data[row * self.dim2..(row + 1) * self.dim2]
    }
}

impl<T> IndexMut<usize> for Matrix2D<T> {
    fn index_mut(&mut self, row: usize) -> &mut [T] {
        assert!(row < self.dim1, "Matrix2D row index out of range");
        &mut self.data[row * self.dim2..(row + 1) * self.dim2]
    }
}

/// `fdkCallocMatrix2D(dim1, dim2, size)`: a zero-initialized `dim1` x `dim2` matrix, `None`
/// (`NULL`) if a dimension is 0.
pub fn fdk_calloc_matrix_2d<T: Clone + Default>(dim1: u32, dim2: u32) -> Option<Matrix2D<T>> {
    if dim1 == 0 || dim2 == 0 {
        return None;
    }
    Some(Matrix2D::new(dim1 as usize, dim2 as usize))
}

/// `fdkCallocMatrix2D_aligned(dim1, dim2, size)`
pub fn fdk_calloc_matrix_2d_aligned<T: Clone + Default>(
    dim1: u32,
    dim2: u32,
) -> Option<Matrix2D<T>> {
    fdk_calloc_matrix_2d(dim1, dim2)
}

/// `fdkCallocMatrix2D_int(dim1, dim2, size, s)`
pub fn fdk_calloc_matrix_2d_int<T: Clone + Default>(
    dim1: u32,
    dim2: u32,
    _s: MemorySection,
) -> Option<Matrix2D<T>> {
    fdk_calloc_matrix_2d(dim1, dim2)
}

/// `fdkCallocMatrix2D_int_aligned(dim1, dim2, size, s)`
pub fn fdk_calloc_matrix_2d_int_aligned<T: Clone + Default>(
    dim1: u32,
    dim2: u32,
    _s: MemorySection,
) -> Option<Matrix2D<T>> {
    fdk_calloc_matrix_2d(dim1, dim2)
}

/// `fdkFreeMatrix2D(p)`
pub fn fdk_free_matrix_2d<T>(_p: Option<Matrix2D<T>>) {}

/// `fdkFreeMatrix2D_aligned(p)`
pub fn fdk_free_matrix_2d_aligned<T>(_p: Option<Matrix2D<T>>) {}

/// A zero-initialized `dim1` x `dim2` x `dim3` matrix with contiguous row-major storage
/// (`fdkCallocMatrix3D`): `p[i][j][k]` is `m[(i, j)][k]`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Matrix3D<T> {
    data: Vec<T>,
    dim1: usize,
    dim2: usize,
    dim3: usize,
}

impl<T: Clone + Default> Matrix3D<T> {
    /// A zero-initialized `dim1` x `dim2` x `dim3` matrix.
    ///
    /// # Panics
    /// If the number of elements overflows `usize` (no such allocation could succeed).
    pub fn new(dim1: usize, dim2: usize, dim3: usize) -> Self {
        let n = dim1
            .checked_mul(dim2)
            .and_then(|n| n.checked_mul(dim3))
            .expect("matrix size overflows usize");
        Matrix3D { data: vec![T::default(); n], dim1, dim2, dim3 }
    }
}

impl<T> Matrix3D<T> {
    /// The first dimension.
    pub fn dim1(&self) -> usize {
        self.dim1
    }

    /// The second dimension.
    pub fn dim2(&self) -> usize {
        self.dim2
    }

    /// The third dimension.
    pub fn dim3(&self) -> usize {
        self.dim3
    }

    /// All elements in row-major order (`p[0][0][0 .. dim1 * dim2 * dim3]` in C).
    pub fn as_flat(&self) -> &[T] {
        &self.data
    }

    /// All elements in row-major order (`p[0][0][0 .. dim1 * dim2 * dim3]` in C).
    pub fn as_flat_mut(&mut self) -> &mut [T] {
        &mut self.data
    }

    /// The `dim2 * dim3` elements of `p[i]`.
    pub fn plane(&self, i: usize) -> &[T] {
        assert!(i < self.dim1, "Matrix3D index out of range");
        let n = self.dim2 * self.dim3;
        &self.data[i * n..(i + 1) * n]
    }

    /// The `dim2 * dim3` elements of `p[i]`, mutable.
    pub fn plane_mut(&mut self, i: usize) -> &mut [T] {
        assert!(i < self.dim1, "Matrix3D index out of range");
        let n = self.dim2 * self.dim3;
        &mut self.data[i * n..(i + 1) * n]
    }

    /// All rows `p[i][j]` in order (gives access to several rows at once).
    pub fn rows_mut(&mut self) -> core::slice::ChunksExactMut<'_, T> {
        self.data.chunks_exact_mut(self.dim3.max(1))
    }

    fn offset(&self, i: usize, j: usize) -> usize {
        assert!(i < self.dim1 && j < self.dim2, "Matrix3D index out of range");
        (i * self.dim2 + j) * self.dim3
    }
}

impl<T> Index<(usize, usize)> for Matrix3D<T> {
    type Output = [T];

    /// Row `p[i][j]`.
    fn index(&self, (i, j): (usize, usize)) -> &[T] {
        let o = self.offset(i, j);
        &self.data[o..o + self.dim3]
    }
}

impl<T> IndexMut<(usize, usize)> for Matrix3D<T> {
    fn index_mut(&mut self, (i, j): (usize, usize)) -> &mut [T] {
        let o = self.offset(i, j);
        let n = self.dim3;
        &mut self.data[o..o + n]
    }
}

/// `fdkCallocMatrix3D(dim1, dim2, dim3, size)`: a zero-initialized matrix, `None` (`NULL`) if a
/// dimension is 0.
pub fn fdk_calloc_matrix_3d<T: Clone + Default>(
    dim1: u32,
    dim2: u32,
    dim3: u32,
) -> Option<Matrix3D<T>> {
    if dim1 == 0 || dim2 == 0 || dim3 == 0 {
        return None;
    }
    Some(Matrix3D::new(dim1 as usize, dim2 as usize, dim3 as usize))
}

/// `fdkCallocMatrix3D_int(dim1, dim2, dim3, size, s)`
pub fn fdk_calloc_matrix_3d_int<T: Clone + Default>(
    dim1: u32,
    dim2: u32,
    dim3: u32,
    _s: MemorySection,
) -> Option<Matrix3D<T>> {
    fdk_calloc_matrix_3d(dim1, dim2, dim3)
}

/// `fdkFreeMatrix3D(p)`
pub fn fdk_free_matrix_3d<T>(_p: Option<Matrix3D<T>>) {}
