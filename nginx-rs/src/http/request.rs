use crate::bindings::*;
use crate::core::*;
use crate::http::status::*;
use std::sync::Arc;

use std::os::raw::c_void;
use std::ptr;

#[macro_export]
macro_rules! http_request_handler {
    ( $x: ident, $m: ident, $y: expr ) => {
        #[no_mangle]
        extern "C" fn $x(r: *mut ngx_http_request_t) -> ngx_int_t {
            let status: Status =
                $y(unsafe { &mut $crate::http::Request::from_ngx_http_request(r, &mut $m) });
            status.0
        }
    };
}

pub struct Request(*mut ngx_http_request_t, *mut ngx_module_t);

impl Request {
    pub unsafe fn from_ngx_http_request(
        r: *mut ngx_http_request_t,
        the_mod: *mut ngx_module_t,
    ) -> Request {
        Request(r, the_mod)
    }

    pub fn is_main(&self) -> bool {
        self.0 == unsafe { (*self.0).main }
    }

    pub fn pool(&self) -> Pool {
        unsafe { Pool::from_ngx_pool((*self.0).pool) }
    }

    pub fn connection(&self) -> *mut ngx_connection_t {
        unsafe { (*self.0).connection }
    }

    pub fn get_module_loc_conf(&self, module: &ngx_module_t) -> *mut c_void {
        unsafe { *(*self.0).loc_conf.add(module.ctx_index) }
    }

    pub fn get_complex_value(&self, cv: &mut ngx_http_complex_value_t) -> Option<NgxStr> {
        let mut res = ngx_str_t {
            len: 0,
            data: ptr::null_mut(),
        };
        unsafe {
            if ngx_http_complex_value(self.0, cv, &mut res) != NGX_OK as ngx_int_t {
                return None;
            }
            Some(NgxStr::from_ngx_str(res))
        }
    }

    pub fn discard_request_body(&mut self) -> Status {
        Status(unsafe { ngx_http_discard_request_body(self.0) })
    }

    pub fn user_agent(&self) -> NgxStr {
        unsafe { NgxStr::from_ngx_str((*(*self.0).headers_in.user_agent).value) }
    }

    pub fn set_status(&mut self, status: HTTPStatus) {
        unsafe {
            (*self.0).headers_out.status = status.into();
        }
    }

    pub fn set_content_length_n(&mut self, n: usize) {
        unsafe {
            (*self.0).headers_out.content_length_n = n as off_t;
        }
    }

    pub fn send_header(&self) -> Status {
        Status(unsafe { ngx_http_send_header(self.0) })
    }

    pub fn set_header_only(&self) -> bool {
        unsafe { (*self.0).header_only() != 0 }
    }

    pub fn output_filter(&mut self, body: &mut ngx_chain_t) -> Status {
        Status(unsafe { ngx_http_output_filter(self.0, body) })
    }

    /// get the optional context variable
    pub fn get_context<T: std::fmt::Debug>(&mut self) -> Option<Arc<T>> {
        unsafe {
            use std::ops::Deref;

            // do a bunch of pointer arithmatic
            let req = self.0;
            let ctx_ptr = (*req).ctx;
            let index = (*self.1).ctx_index;

            let pointer = ptr::slice_from_raw_parts(ctx_ptr, index + 1);
            let item = (&*pointer)[index];
            // and some casting
            let item = item as *mut Arc<T>;

            // if it's a NULL, return None
            if std::ptr::null_mut() == item {
                return None;
            }

            // put the pointer in a Box
            let owned = Box::from_raw(item);

            // deref the box to the get Arc and clone the Arc
            let deboxed_and_cloned: Arc<T> = owned.deref().clone();
            // release the box from ownership of the pointer... it'll be cleaned
            // up by the drop function
            let _dont_double_free = Box::into_raw(owned);

            // return the cloned Arc
            Some(deboxed_and_cloned)
        }
    }

    pub fn set_context<T: std::fmt::Debug>(&mut self, info: &Arc<T>) {
        unsafe {
            // deal with a clone of the Arc
            let cloned_info = info.clone();

            // put it in a box and then turn the Box into a Raw
            let ptr = Box::into_raw(Box::from(cloned_info)) as *mut std::ffi::c_void;
            // do a bunch of things to simulate pointer arithmatic
            let req = self.0;
            let ctx_ptr = (*req).ctx;
            let index = (*self.1).ctx_index;

            let pointer = ptr::slice_from_raw_parts_mut(ctx_ptr, index + 1);
            (*pointer)[index] = ptr;

            // make sure we clean up the thing
            // this is a kind hack-tastic thingy...
            // we connect to the Nginx pool cleanup code... which
            // will "do the right thing" based on specialization (a different
            // function for each type)
            let pca = ngx_pool_cleanup_add((*self.0).pool, 0);
            (*pca).data = ptr;
            (*pca).handler =
                Some(drop_context_ptr::<T> as unsafe extern "C" fn(*mut std::ffi::c_void));
        }
    }
}

pub unsafe extern "C" fn drop_context_ptr<T: std::fmt::Debug>(v: *mut std::ffi::c_void) {
    if v != std::ptr::null_mut() {
        let _q: Box<Arc<T>> = Box::from_raw(v as *mut Arc<T>);

        // the box will be dropped and the enclosed thing will be
        // dropped
    }
}
