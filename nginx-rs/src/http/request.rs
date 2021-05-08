use crate::bindings::*;
use crate::core::*;
use crate::http::status::*;
use async_trait::async_trait;
use lazy_static::lazy_static;
use std::collections::VecDeque;
use std::os::raw::c_void;
use std::ptr;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::{channel, Receiver, Sender};

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

#[macro_export]
macro_rules! http_abort_handler {
    ( $x: ident, $m: ident, $y: expr ) => {
        #[no_mangle]
        extern "C" fn $x(r: *mut ngx_http_request_t) {
            ($y as fn(&mut Request))(unsafe {
                &mut $crate::http::Request::from_ngx_http_request(r, &mut $m)
            });
        }
    };
}

#[macro_export]
macro_rules! http_finalize_handler {
    ( $x: ident, $m: ident, $y: expr ) => {
        #[no_mangle]
        extern "C" fn $x(r: *mut ngx_http_request_t, i: ngx_int_t) {
            ($y as fn(&mut Request, ngx_int_t))(
                unsafe { &mut $crate::http::Request::from_ngx_http_request(r, &mut $m) },
                i,
            );
        }
    };
}

#[macro_export]
macro_rules! http_event_handler {
    ( $x: ident, $m: ident, $y: expr ) => {
        #[no_mangle]
        extern "C" fn $x(r: *mut ngx_http_request_t, u: *mut ngx_http_upstream_t) {
            ($y as fn(&mut Request, &mut Upstream))(
                unsafe { &mut $crate::http::Request::from_ngx_http_request(r, &mut $m) },
                unsafe { &mut $crate::http::Upstream::new(u, &mut $m) },
            );
        }
    };
}

pub type RequestFunc = unsafe extern "C" fn(*mut ngx_http_request_t) -> ngx_int_t;
pub type AbortRequestFunc = unsafe extern "C" fn(*mut ngx_http_request_t);
pub type FinalizeRequestFunc = unsafe extern "C" fn(*mut ngx_http_request_t, ngx_int_t);
pub type EventHandlerFunc =
    unsafe extern "C" fn(r: *mut ngx_http_request_t, u: *mut ngx_http_upstream_t);
pub type InputFilterInitFunc = unsafe extern "C" fn(data: *mut ::std::os::raw::c_void) -> ngx_int_t;
pub type InputFilterFunc =
    unsafe extern "C" fn(data: *mut ::std::os::raw::c_void, bytes: i64) -> ngx_int_t;

pub struct Upstream {
    up: *mut ngx_http_upstream_t,
    _module: *mut ngx_module_t,
    // func: Vec<Option<RequestFunc>>
}

impl Upstream {
    pub fn new(u: *mut ngx_http_upstream_t, the_mod: *mut ngx_module_t) -> Upstream {
        Upstream {
            up: u,
            _module: the_mod,
        }
    }

    pub fn set_create_request(&mut self, request: Option<RequestFunc>) {
        unsafe {
            (*self.up).create_request = request;
        }
    }
    pub fn set_reinit_request(&mut self, request: Option<RequestFunc>) {
        unsafe {
            (*self.up).reinit_request = request;
        }
    }
    pub fn set_output_tag(&mut self, tag: *const ngx_module_t) {
        unsafe {
            (*self.up).output.tag = tag as *mut std::ffi::c_void;
        }
    }
    pub fn set_process_header(&mut self, request: Option<RequestFunc>) {
        unsafe {
            (*self.up).process_header = request;
        }
    }
    pub fn set_abort_request(&mut self, request: Option<AbortRequestFunc>) {
        unsafe {
            (*self.up).abort_request = request;
        }
    }
    pub fn set_finalize_request(&mut self, request: Option<FinalizeRequestFunc>) {
        unsafe {
            (*self.up).finalize_request = request;
        }
    }

    pub fn set_read_event_handler(&mut self, handler: Option<EventHandlerFunc>) {
        unsafe {
            (*self.up).read_event_handler = handler;
        }
    }
    pub fn set_write_event_handler(&mut self, handler: Option<EventHandlerFunc>) {
        unsafe {
            (*self.up).write_event_handler = handler;
        }
    }
    pub fn set_log_error(&mut self, level: u32) {
        unsafe {
            (*self.up).peer.set_log_error(level);
        }
    }

    pub fn set_input_filter(&mut self, handler: Option<InputFilterFunc>) {
        unsafe {
            (*self.up).input_filter = handler;
        }
    }

    pub fn set_input_filter_init(&mut self, handler: Option<InputFilterInitFunc>) {
        unsafe {
            (*self.up).input_filter_init = handler;
        }
    }

    // u->input_filter_ctx = NULL;

    pub fn set_schema(&mut self, schema: &'static str) {
        unsafe {
            let s = &mut (*self.up).schema;
            (*s).len = schema.len() as u64;
            (*s).data = schema.as_ptr() as *mut u8;
        }
    }

    // u->output.tag = (ngx_buf_tag_t) &ngx_http_drizzle_module;

    pub fn set_conf(&mut self, conf: *mut ngx_http_upstream_conf_t) {
        unsafe {
            (*self.up).conf = conf;
        }
    }
}

#[async_trait]
pub trait WorkerFunc: Send + Sync {
    async fn run(&self, r: Request);
}

pub type WorkerBox = Arc<dyn WorkerFunc>;

fn start_loop(mut rx: Receiver<(Request, WorkerBox)>) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(10)
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async move {
            loop {
                match rx.recv().await {
                    Some((request, the_func)) => {
                        tokio::spawn(async move {
                            the_func.run(request).await;

                            match REQUEST_QUEUE.lock() {
                                Ok(mut q) => q.push_back(request),
                                _ => (),
                            };

                            unsafe {
                                match ngx_event_actions.notify {
                                    Some(func) => {
                                        (func)(Some(finish_loop));
                                        ()
                                    }
                                    None => (),
                                }
                            };
                        });
                    }
                    _ => break,
                }
            }
        });
    });
}

lazy_static! {
    static ref SEND_JOB: Mutex<Sender<(Request, WorkerBox)>> = {
        let (tx, rx) = channel(10000);
        start_loop(rx);
        Mutex::new(tx)
    };
    static ref REQUEST_QUEUE: Mutex<VecDeque<Request>> = Mutex::new(VecDeque::new());
}

unsafe extern "C" fn finish_loop(_ev: *mut ngx_event_t) {
    loop {
        match REQUEST_QUEUE.lock() {
            Ok(mut q) => match q.pop_front() {
                Some(mut r) => {
                    r.dec_blocked();
                    // don't run the request if, somehow, the pool has been nulled
                    if (*r.get_request()).pool != std::ptr::null_mut() {
                        ngx_http_handler(r.get_request());
                    }
                }
                None => break,
            },
            _ => (),
        }
    }
}
#[macro_export]
macro_rules! build_worker {
    ( $r: ident, $y: block ) => {{
        use async_trait::async_trait;
        struct MyWorker {}
        #[async_trait]
        impl $crate::http::WorkerFunc for MyWorker {
            async fn run(&self, r: Request) {
                let $r = r;
                $y;
                ()
            }
        }

        Arc::new(MyWorker {})
    }};
}

#[derive(Clone, Copy, Debug)]
pub struct Request {
    request: *mut ngx_http_request_t,
    module: *mut ngx_module_t,
    the_pool: Pool,
}

unsafe impl Send for Request {}

unsafe impl Sync for Request {}

//#define ngx_http_get_module_srv_conf(r, module)  (r)->srv_conf[module.ctx_index]
//#define ngx_http_get_module_loc_conf(r, module)  (r)->loc_conf[module.ctx_index]

pub fn ngx_http_get_module_loc_conf(
    req: *mut ngx_http_request_t,
    module: *mut ngx_module_t,
) -> *mut ngx_http_upstream_conf_t {
    unsafe {
        let ctx_ptr = (*req).loc_conf;
        let index = (*module).ctx_index;

        let pointer = ptr::slice_from_raw_parts(ctx_ptr, index + 1);
        let item = (&*pointer)[index];
        item as *mut ngx_http_upstream_conf_t
    }
}

impl Request {
    pub unsafe fn from_ngx_http_request(
        r: *mut ngx_http_request_t,
        the_mod: *mut ngx_module_t,
    ) -> Request {
        Request {
            request: r,
            module: the_mod,
            the_pool: Pool::from_ngx_pool((*r).pool),
        }
    }

    pub fn run_in_background<T: WorkerFunc + 'static>(
        &mut self,
        background_func: Arc<T>,
    ) -> Status {
        self.inc_blocked();
        match SEND_JOB
            .lock()
            .unwrap()
            .blocking_send((self.clone(), background_func.clone()))
        {
            Ok(_) => (),
            _ => panic!("Failed blocking send"),
        }

        NGX_AGAIN.into()
    }

    pub fn get_loc_conf(&mut self) -> *mut ngx_http_upstream_conf_t {
        ngx_http_get_module_loc_conf(self.request, self.module)
    }

    pub fn create_upstream(&mut self) -> Upstream {
        unsafe {
            let u = self
                .the_pool
                .calloc(std::mem::size_of::<ngx_http_upstream_t>() as u64)
                as *mut ngx_http_upstream_t;

            (*u).peer.log = (*(*self.request).connection).log;
            (*u).peer
                .set_log_error(ngx_connection_log_error_e_NGX_ERROR_ERR);
            let mut ret = Upstream {
                up: u,
                _module: self.module,
            };
            let plcf = self.get_loc_conf();
            ret.set_conf(plcf);

            ret
        }
    }

    pub fn get_upstream(&mut self) -> Option<Upstream> {
        let u = unsafe { (*self.request).upstream };
        if u == std::ptr::null_mut() {
            None
        } else {
            Some(Upstream {
                up: u,
                _module: self.module,
            })
        }
    }

    pub fn is_main(&self) -> bool {
        self.request == unsafe { (*self.request).main }
    }

    pub fn get_request(&self) -> *mut ngx_http_request_t {
        self.request
    }
    pub fn pool<'a>(&'a mut self) -> &'a mut Pool {
        &mut self.the_pool
    }

    pub fn set_upstream(&mut self, upstream: &Upstream) {
        unsafe {
            (*self.request).upstream = upstream.up;
        }
    }

    pub fn connection(&self) -> *mut ngx_connection_t {
        unsafe { (*self.request).connection }
    }

    pub fn get_main_count(&self) -> u32 {
        unsafe {
            let main = (*self.request).main;
            (*main).count()
        }
    }

    pub fn inc_main_count(&mut self) -> u32 {
        unsafe {
            let main = (*self.request).main;
            let mut cc = (*main).count();
            cc += 1;
            (*main).set_count(cc);
            cc
        }
    }

    pub fn dec_main_count(&mut self) -> u32 {
        unsafe {
            let main = (*self.request).main;
            let mut cc = (*main).count();
            if cc > 0 {
                cc -= 1;
                (*main).set_count(cc);
            }
            cc
        }
    }

    pub fn inc_blocked(&mut self) -> u32 {
        unsafe {
            let main = (*self.request).main;
            let mut cc = (*main).blocked();
            cc += 1;
            (*main).set_blocked(cc);
            cc
        }
    }

    pub fn get_blocked_count(&self) -> u32 {
        unsafe {
            let main = (*self.request).main;
            (*main).blocked()
        }
    }

    pub fn dec_blocked(&mut self) -> u32 {
        unsafe {
            let main = (*self.request).main;
            let mut cc = (*main).blocked();
            if cc > 0 {
                cc -= 1;
                (*main).set_blocked(cc);
            }
            cc
        }
    }

    pub fn get_module_loc_conf(&self, module: &ngx_module_t) -> *mut c_void {
        unsafe { *(*self.request).loc_conf.add(module.ctx_index) }
    }

    pub fn get_complex_value(&self, cv: &mut ngx_http_complex_value_t) -> Option<NgxStr> {
        let mut res = ngx_str_t {
            len: 0,
            data: ptr::null_mut(),
        };
        unsafe {
            if ngx_http_complex_value(self.request, cv, &mut res) != NGX_OK as ngx_int_t {
                return None;
            }
            Some(NgxStr::from_ngx_str(res))
        }
    }

    pub fn discard_request_body(&mut self) -> Status {
        Status(unsafe { ngx_http_discard_request_body(self.request) })
    }

    pub fn user_agent(&self) -> NgxStr {
        unsafe { NgxStr::from_ngx_str((*(*self.request).headers_in.user_agent).value) }
    }

    pub fn set_status(&mut self, status: HTTPStatus) {
        unsafe {
            (*self.request).headers_out.status = status.into();
        }
    }

    pub fn set_content_length_n(&mut self, n: usize) {
        unsafe {
            (*self.request).headers_out.content_length_n = n as off_t;
        }
    }

    pub fn send_header(&self) -> Status {
        Status(unsafe { ngx_http_send_header(self.request) })
    }

    pub fn set_header_only(&self) -> bool {
        unsafe { (*self.request).header_only() != 0 }
    }

    pub fn output_filter(&mut self, body: &mut ngx_chain_t) -> Status {
        Status(unsafe { ngx_http_output_filter(self.request, body) })
    }

    /// Get the context variable... if it has not been set, build a new context from
    /// the function and set the context. Returns the context and false if the context had
    /// already been set and true if the context was initialized
    pub fn get_or_init_context<T: std::fmt::Debug, F: Fn() -> T>(
        &mut self,
        init_func: F,
    ) -> (Arc<T>, bool) {
        match self.get_context::<T>() {
            Some(v) => (v, false),
            None => {
                let ret = Arc::new(init_func());
                self.set_context(&ret);
                (ret, true)
            }
        }
    }
    /// get the optional context variable
    pub fn get_context<T: std::fmt::Debug>(&self) -> Option<Arc<T>> {
        unsafe {
            use std::ops::Deref;

            // do a bunch of pointer arithmatic
            let req = self.request;
            let ctx_ptr = (*req).ctx;
            let index = (*self.module).ctx_index;

            let pointer = ptr::slice_from_raw_parts(ctx_ptr, index + 1);
            let item = (&*pointer)[index];

            // if it's a NULL, return None
            if std::ptr::null_mut() == item {
                return None;
            }

            // and some casting
            let item = item as *mut Arc<T>;
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
            let req = self.request;
            let ctx_ptr = (*req).ctx;
            let index = (*self.module).ctx_index;

            let pointer = ptr::slice_from_raw_parts_mut(ctx_ptr, index + 1);
            (*pointer)[index] = ptr;

            // make sure we clean up the thing
            // this is a kind hack-tastic thingy...
            // we connect to the Nginx pool cleanup code... which
            // will "do the right thing" based on specialization (a different
            // function for each type)
            let pca = ngx_pool_cleanup_add((*self.request).pool, 0);
            (*pca).data = ptr;
            (*pca).handler = Some(drop_context_ptr::<T> as unsafe extern "C" fn(*mut c_void));

            let pca = ngx_pool_cleanup_add((*self.request).pool, 0);
            let p2 = ctx_ptr.offset(index as isize);
            (*pca).data = p2 as *mut c_void;
            (*pca).handler = Some(null_ptr as unsafe extern "C" fn(*mut c_void));
        }
    }
}

pub unsafe extern "C" fn drop_context_ptr<T: std::fmt::Debug>(v: *mut c_void) {
    if v != std::ptr::null_mut() {
        let _q: Box<Arc<T>> = Box::from_raw(v as *mut Arc<T>);
        // the box will be dropped and the enclosed thing will be
        // dropped
    }
}

pub unsafe extern "C" fn null_ptr(v: *mut c_void) {
    if v != std::ptr::null_mut() {
        // null out the context on the request
        let v2 = v as *mut *mut c_void;
        (*v2) = std::ptr::null_mut();
    }
}
