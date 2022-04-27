use bytes::Bytes;
use futures::{Sink, Stream};
use futures_util::SinkExt;
use std::error::Error;
use std::future::ready;

use super::proto::{Request, Response};
