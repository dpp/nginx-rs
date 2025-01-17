use crate::bindings::*;

use std::borrow::Cow;
use std::marker::PhantomData;
use std::slice;
use std::str::{self, Utf8Error};

pub struct NgxStr<'a>(ngx_str_t, PhantomData<&'a [u8]>);

impl<'a> NgxStr<'a> {
    pub fn new(str: &str) -> NgxStr {
        NgxStr(
            ngx_str_t {
                len: str.len() as u64,
                data: str.as_ptr() as *mut u_char,
            },
            PhantomData,
        )
    }

    pub unsafe fn from_ngx_str(str: ngx_str_t) -> NgxStr<'a> {
        NgxStr(str, PhantomData)
    }

    pub fn as_bytes(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.0.data, self.0.len as usize) }
    }

    pub fn to_str(&self) -> Result<&str, Utf8Error> {
        str::from_utf8(self.as_bytes())
    }

    pub fn to_string_lossy(&self) -> Cow<str> {
        String::from_utf8_lossy(self.as_bytes())
    }

    pub fn is_empty(&self) -> bool {
        self.0.len == 0
    }
}

impl AsRef<[u8]> for NgxStr<'_> {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl Default for NgxStr<'_> {
    fn default() -> Self {
        NgxStr(
            ngx_str_t {
                len: 0,
                data: b"".as_ptr() as *mut u_char,
            },
            PhantomData,
        )
    }
}
