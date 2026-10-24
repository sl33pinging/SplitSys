//! Minimal dense row-major matrices.

/// Dense row-major matrix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mat<T> {
    rows: usize,
    cols: usize,
    data: Vec<T>,
}

/// int8 matrix (A, W, requantized C).
pub type MatI8 = Mat<i8>;
/// int32 matrix (accumulators).
pub type MatI32 = Mat<i32>;

impl<T: Copy + Default> Mat<T> {
    /// All-default (zero) matrix.
    pub fn zeros(rows: usize, cols: usize) -> Self {
        Self {
            rows,
            cols,
            data: vec![T::default(); rows * cols],
        }
    }

    /// Build from a row-major vector. Panics if the length is wrong.
    pub fn from_vec(rows: usize, cols: usize, data: Vec<T>) -> Self {
        assert_eq!(data.len(), rows * cols, "Mat::from_vec: length mismatch");
        Self { rows, cols, data }
    }

    /// Build element by element: `f(row, col)`.
    pub fn from_fn(rows: usize, cols: usize, mut f: impl FnMut(usize, usize) -> T) -> Self {
        let mut data = Vec::with_capacity(rows * cols);
        for r in 0..rows {
            for c in 0..cols {
                data.push(f(r, c));
            }
        }
        Self { rows, cols, data }
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Row-major element slice.
    pub fn data(&self) -> &[T] {
        &self.data
    }

    pub fn get(&self, r: usize, c: usize) -> T {
        assert!(r < self.rows && c < self.cols, "Mat::get out of bounds");
        self.data[r * self.cols + c]
    }

    pub fn set(&mut self, r: usize, c: usize, v: T) {
        assert!(r < self.rows && c < self.cols, "Mat::set out of bounds");
        self.data[r * self.cols + c] = v;
    }

    pub fn row(&self, r: usize) -> &[T] {
        &self.data[r * self.cols..(r + 1) * self.cols]
    }

    /// Rows as nested vectors (handy for JSON).
    pub fn to_rows(&self) -> Vec<Vec<T>> {
        (0..self.rows).map(|r| self.row(r).to_vec()).collect()
    }

    /// The first `rows` rows.
    pub fn top_rows(&self, rows: usize) -> Self {
        assert!(rows <= self.rows);
        Self::from_vec(rows, self.cols, self.data[..rows * self.cols].to_vec())
    }
}
