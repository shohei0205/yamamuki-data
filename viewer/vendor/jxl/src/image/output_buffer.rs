// Copyright (c) the JPEG XL Project Authors. All rights reserved.
//
// Use of this source code is governed by a BSD-style
// license that can be found in the LICENSE file.

use std::fmt::Debug;
use std::marker::PhantomData;

use super::internal::RawImageBuffer;
use super::{RawImageRectMut, Rect};
use crate::image::ImageDataType;

#[derive(Debug)]

pub struct JxlOutputBuffer<'a> {
    // Safety invariant: `self` has exclusive (write) access to the accessible bytes of `inner`.
    inner: Option<RawImageBuffer>,
    sink: Option<&'a dyn JxlRowSink>,
    sink_rect: Rect,
    _ph: PhantomData<&'a mut u8>,
}

impl<'a> JxlOutputBuffer<'a> {
    /// 向き補正なし・8bitグレースケールの行出力専用。
    pub fn new_gray_sink(sink: &'a dyn JxlRowSink, width: usize, height: usize) -> Self {
        Self {
            inner: None,
            sink: Some(sink),
            sink_rect: Rect {
                origin: (0, 0),
                size: (width, height),
            },
            _ph: PhantomData,
        }
    }
    pub(crate) fn is_row_sink(&self) -> bool {
        self.sink.is_some()
    }
    pub(crate) fn write_gray_row(&self, row: usize, pixels: &[u8]) {
        assert!(row < self.sink_rect.size.1 && pixels.len() <= self.sink_rect.size.0);
        self.sink.expect("行出力が必要です").write_row(
            self.sink_rect.origin.0,
            self.sink_rect.origin.1 + row,
            pixels,
        );
    }

    /// Creates a new JxlOutputBuffer from raw pointers.
    ///
    /// # Safety
    /// - `buf` must be valid for writes for all bytes in the range
    ///   `buf[i*bytes_between_rows..i*bytes_between_rows+bytes_per_row]` for all values of `i`
    ///   from `0` to `num_rows-1`.
    /// - `buf` must be initialized for all the bytes in these ranges.
    /// - The bytes in these ranges must not be accessed as long as the returned `Self` is in scope.
    /// - All the bytes in those ranges (and in between) must be part of the same allocated object.
    pub unsafe fn new_from_ptr(
        buf: *mut u8,
        num_rows: usize,
        bytes_per_row: usize,
        bytes_between_rows: usize,
    ) -> Self {
        JxlOutputBuffer {
            // SAFETY: the safety conditions on RawImageBuffer::new_from_ptr are strictly weaker.
            // We are promised write access to the underlying data, so our own safety invariant is
            // respected.
            inner: Some(unsafe {
                RawImageBuffer::new_from_ptr(
                    buf.cast(),
                    num_rows,
                    bytes_per_row,
                    bytes_between_rows,
                )
            }),
            sink: None,
            sink_rect: Rect {
                origin: (0, 0),
                size: (0, 0),
            },
            _ph: PhantomData,
        }
    }

    pub fn from_image_rect_mut(raw: RawImageRectMut<'a>) -> Self {
        Self {
            // Safety note: since `raw` has exclusive access to the data, we are just transferring
            // this access.
            inner: Some(raw.data),
            sink: None,
            sink_rect: Rect {
                origin: (0, 0),
                size: (0, 0),
            },
            _ph: PhantomData,
        }
    }

    pub fn new_with_stride(
        buf: &'a mut [u8],
        num_rows: usize,
        bytes_per_row: usize,
        byte_stride: usize,
    ) -> Self {
        assert_ne!(num_rows, 0);
        assert!(
            buf.len()
                >= byte_stride
                    .checked_mul(num_rows - 1)
                    .unwrap()
                    .checked_add(bytes_per_row)
                    .unwrap()
        );
        // SAFETY: The assert above guarantees that `buf` has enough space to satisfy the first
        // safety requirement, and the rest follow from borrowing from a &mut [].
        unsafe { Self::new_from_ptr(buf.as_mut_ptr(), num_rows, bytes_per_row, byte_stride) }
    }

    pub fn new(buf: &'a mut [u8], num_rows: usize, bytes_per_row: usize) -> Self {
        Self::new_with_stride(buf, num_rows, bytes_per_row, bytes_per_row)
    }

    pub(crate) fn reborrow(lender: &'a mut JxlOutputBuffer<'_>) -> JxlOutputBuffer<'a> {
        // Safety note: this is effectively equivalent to a reborrow.
        Self {
            _ph: PhantomData,
            ..*lender
        }
    }

    pub(crate) fn typed_row_mut<T: ImageDataType>(&mut self, row: usize) -> &mut [T] {
        T::cast_slice_mut(self.row_mut(row))
    }

    pub(crate) fn row_mut(&mut self, row: usize) -> &mut [u8] {
        // SAFETY: we have write access to the data due to safety invariant.
        unsafe {
            self.inner
                .as_mut()
                .expect("行コールバックは直接書き込みできません")
                .row_mut(row)
        }
    }

    pub fn byte_size(&self) -> (usize, usize) {
        self.inner
            .as_ref()
            .map_or(self.sink_rect.size, |b| b.byte_size())
    }

    /// # Safety
    /// The caller must guarantee that there are no outstanding lends
    /// of the provided `rect`.
    pub(crate) unsafe fn rect(&self, rect: Rect) -> JxlOutputBuffer<'_> {
        // Safety note: the return value borrows from `self`, so we are lending our memory to the
        // returned JxlOutputBuffer. The caller guarantees that the region identified by
        // `rect` does not overlap with other regions with outstanding borrows, so we have
        // exclusive access to that region.
        rect.check_within(self.byte_size());
        Self {
            inner: self.inner.as_ref().map(|b| b.rect(rect)),
            sink: self.sink,
            sink_rect: Rect {
                origin: (
                    self.sink_rect.origin.0 + rect.origin.0,
                    self.sink_rect.origin.1 + rect.origin.1,
                ),
                size: rect.size,
            },
            _ph: PhantomData,
        }
    }
}

/// 原寸ラスタを確保せず、復号したグレースケール行を受け取る。
pub trait JxlRowSink: Debug + Send + Sync {
    fn write_row(&self, x: usize, y: usize, pixels: &[u8]);
}
