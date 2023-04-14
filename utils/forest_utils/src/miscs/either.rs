// Copyright 2019-2023 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::AsyncRead;

pub enum Either<L, R> {
    Left(L),
    Right(R),
}

impl<L, R> Either<L, R> {
    fn left_mut(&mut self) -> Option<&mut L> {
        match self {
            Self::Left(left) => Some(left),
            _ => None,
        }
    }

    fn right_mut(&mut self) -> Option<&mut R> {
        match self {
            Self::Right(right) => Some(right),
            _ => None,
        }
    }
}

impl<L: AsyncRead + Unpin, R: AsyncRead + Unpin> AsyncRead for Either<L, R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.deref_mut() {
            Self::Left(left) => Pin::new(left).poll_read(cx, buf),
            Self::Right(right) => Pin::new(right).poll_read(cx, buf),
        }
    }
}
