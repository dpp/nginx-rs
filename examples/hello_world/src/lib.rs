use nginx_rs::bindings::*;
use nginx_rs::core::*;
use nginx_rs::http::*;

use nginx_rs::{
    build_worker, http_request_handler, ngx_log_debug_http, ngx_modules, ngx_null_command,
    ngx_string,
};
use reqwest::get;
use std::borrow::Cow;
use std::os::raw::{c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;

#[no_mangle]
static mut ngx_http_hello_world_commands: [ngx_command_t; 3] = [
    ngx_command_t {
        name: ngx_string!(b"hello_world\0"),
        type_: (NGX_HTTP_LOC_CONF | NGX_CONF_NOARGS) as ngx_uint_t,
        set: Some(ngx_http_hello_world),
        conf: 0,
        offset: 0,
        post: ptr::null_mut(),
    },
    ngx_command_t {
        name: ngx_string!(b"hello_world_text\0"),
        type_: (NGX_HTTP_LOC_CONF | NGX_CONF_TAKE1) as ngx_uint_t,
        set: Some(ngx_http_hello_world_set_text),
        conf: NGX_RS_HTTP_LOC_CONF_OFFSET as usize,
        offset: 0,
        post: ptr::null_mut(),
    },
    ngx_null_command!(),
];

#[no_mangle]
static ngx_http_hello_world_module_ctx: ngx_http_module_t = ngx_http_module_t {
    preconfiguration: Some(Module::preconfiguration),
    postconfiguration: Some(Module::postconfiguration),

    create_main_conf: Some(Module::create_main_conf),
    init_main_conf: Some(Module::init_main_conf),

    create_srv_conf: Some(Module::create_srv_conf),
    merge_srv_conf: Some(Module::merge_srv_conf),

    create_loc_conf: Some(Module::create_loc_conf),
    merge_loc_conf: Some(Module::merge_loc_conf),
};

#[no_mangle]
pub static mut ngx_http_hello_world_module: ngx_module_t = ngx_module_t {
    ctx_index: ngx_uint_t::max_value(),
    index: ngx_uint_t::max_value(),
    name: ptr::null_mut(),
    spare0: 0,
    spare1: 0,
    version: nginx_version as ngx_uint_t,
    signature: NGX_RS_MODULE_SIGNATURE.as_ptr() as *const c_char,

    ctx: &ngx_http_hello_world_module_ctx as *const _ as *mut _,
    commands: unsafe { &ngx_http_hello_world_commands[0] as *const _ as *mut _ },
    type_: NGX_HTTP_MODULE as ngx_uint_t,

    init_master: None,
    init_module: None,
    init_process: None,
    init_thread: None,
    exit_thread: None,
    exit_process: None,
    exit_master: None,

    spare_hook0: 0,
    spare_hook1: 0,
    spare_hook2: 0,
    spare_hook3: 0,
    spare_hook4: 0,
    spare_hook5: 0,
    spare_hook6: 0,
    spare_hook7: 0,
};

ngx_modules!(ngx_http_hello_world_module);

struct Module;

impl HTTPModule for Module {
    type MainConf = ();
    type SrvConf = ();
    type LocConf = LocConf;

    unsafe extern "C" fn postconfiguration(cf: *mut ngx_conf_t) -> ngx_int_t {
        let cmcf = ngx_http_conf_get_module_main_conf(cf, &ngx_http_core_module)
            as *mut ngx_http_core_main_conf_t;

        let h = ngx_array_push(
            &mut (*cmcf).phases[ngx_http_phases_NGX_HTTP_ACCESS_PHASE as usize].handlers,
        ) as *mut ngx_http_handler_pt;
        if h.is_null() {
            return ERROR.into();
        }

        *h = Some(ngx_http_hello_world_access_handler);

        OK.into()
    }
}

#[derive(Default)]
struct LocConf {
    text: String,
}

impl Merge for LocConf {
    fn merge(&mut self, prev: &LocConf) {
        if self.text.is_empty() {
            self.text = String::from(if !prev.text.is_empty() {
                &prev.text
            } else {
                ""
            });
        }
    }
}

#[no_mangle]
unsafe extern "C" fn ngx_http_hello_world(
    cf: *mut ngx_conf_t,
    _cmd: *mut ngx_command_t,
    conf: *mut c_void,
) -> *mut c_char {
    let _conf = &mut *(conf as *mut LocConf);
    let clcf = ngx_http_conf_get_module_loc_conf(cf, &ngx_http_core_module)
        as *mut ngx_http_core_loc_conf_t;
    (*clcf).handler = Some(ngx_http_hello_world_handler);

    ptr::null_mut()
}

#[no_mangle]
unsafe extern "C" fn ngx_http_hello_world_set_text(
    cf: *mut ngx_conf_t,
    _cmd: *mut ngx_command_t,
    conf: *mut c_void,
) -> *mut c_char {
    let conf = &mut *(conf as *mut LocConf);
    let args = (*(*cf).args).elts as *mut ngx_str_t;
    let value = NgxStr::from_ngx_str(*args.add(1));
    conf.text = String::from(value.to_string_lossy());

    ptr::null_mut()
}
#[derive(Debug)]
struct HelloContext {
    count: AtomicUsize,
    allow: AtomicBool,
    fetched: Mutex<Option<String>>,
}

impl HelloContext {
    pub fn new() -> HelloContext {
        HelloContext {
            count: AtomicUsize::new(0),
            allow: AtomicBool::new(false),
            fetched: Mutex::new(None),
        }
    }

    pub fn get_count(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }

    pub fn inc_count(&self) -> usize {
        self.count.fetch_add(1, Ordering::Relaxed);
        self.get_count()
    }

    pub fn get_allow(&self) -> bool {
        self.allow.load(Ordering::Relaxed)
    }

    pub fn set_allow(&self, a: bool) {
        self.allow.store(a, Ordering::Relaxed);
    }

    pub fn get_fetched(&self) -> Option<String> {
        self.fetched.lock().unwrap().clone()
    }

    pub fn set_fetched(&self, st: Option<String>) {
        let mut data = self.fetched.lock().unwrap();
        *data = st.clone();
    }

    pub async fn fetch_info(&self) {
        // an example of making an HTTP request in the Async thread. This
        // could be a REST call, a gRPC call, or anything else that can be
        // done with Rust Async
        let resp: reqwest::Response = get("https://httpbin.org/ip").await.unwrap();
        let s = format!("{}", resp.text().await.unwrap());
        self.set_fetched(Some(s));
    }
}

http_request_handler!(
    ngx_http_hello_world_access_handler,
    ngx_http_hello_world_module,
    |request: &mut Request| {
        ngx_log_debug_http!(request, "http hello_world access");

        // get the context information
        let (info, _) = request.get_or_init_context(|| HelloContext::new());
        let v = info.get_count();

        // if we've run the external stuff for this request less than 20 times,
        // send a function to Async/Tokio land for processing
        if v < 20 {
            // run_in_background can only be called from an access handler
            // it takes an async block and hands it off to another thread.
            // the return value is NGX_AGAIN... and the run_in_background
            // increments the `blocked` counter on the Nginx request.
            // this combination leads to the access handler being called
            // again after the async block is run
            return request.run_in_background(
                // the `build_worker` macro takes the variable name of the
                // `Request` struct and a block of async code and constructs
                // all the information needed to hand stuff off to Async-land
                // once the block is completed, the Async handler signals
                // the Nginx worker thread to re-run the request and the
                // function is called again.
                //
                // The only way to communicate between the Nginx worker thread
                // (which should not be blocked) and Async-land is via the context
                // object. The Context object should be a struct that supports inner
                // mutability. In this example, both Atomics and Mutexs are
                // used to support thread-safe inner mutability
                build_worker!(request, {
                    // get the context information
                    if let Some(q2) = request.get_context::<HelloContext>() {
                        // if the count is 1, do an HTTP request
                        if q2.get_count() == 1 {
                            q2.fetch_info().await;
                        }
                        // increment the count
                        q2.inc_count();

                        // we're recomputing the access each time through the loop... not
                        // necessary, but why not?
                        q2.set_allow(!request.user_agent().as_bytes().starts_with(b"curl"));
                    }

                    // a nice async sleep
                    sleep(Duration::from_millis(22)).await;
                }),
            );

            // Also, note that the `run_in_background` scheme can only be used during the
            // access phase of Nginx processing... dpp (David Pollak) is not sure why...
        }
        if !info.get_allow() {
            return HTTP_FORBIDDEN.into();
        }

        OK
    }
);

http_request_handler!(
    ngx_http_hello_world_handler,
    ngx_http_hello_world_module,
    |request: &mut Request| {
        ngx_log_debug_http!(request, "http hello_world handler");

        // Ignore client request body if any
        if !request.discard_request_body().is_ok() {
            return HTTP_INTERNAL_SERVER_ERROR.into();
        }

        let (info, _) = request.get_or_init_context(|| HelloContext::new());

        let extra_str: String = info.get_fetched().unwrap_or("No Info".into());

        let hlcf =
            unsafe { request.get_module_loc_conf(&ngx_http_hello_world_module) as *mut LocConf };
        let text = unsafe { &(*hlcf).text };

        // Create body
        let user_agent = request.user_agent();
        let body = format!(
            "Hello, {}!
            \nCount {}
            \nInfo fetched {}\n",
            if text.is_empty() {
                user_agent.to_string_lossy()
            } else {
                Cow::from(text)
            },
            info.get_count(),
            extra_str,
        );

        // Send header
        request.set_status(HTTP_OK);
        request.set_content_length_n(body.len());
        let status = request.send_header();
        if status == ERROR || status > OK || request.set_header_only() {
            return status;
        }

        // Send body
        let mut buf = match request.pool().create_buffer_from_str(&body) {
            Some(buf) => buf,
            None => return HTTP_INTERNAL_SERVER_ERROR.into(),
        };
        assert!(&buf.as_bytes()[..7] == b"Hello, ");
        buf.set_last_buf(request.is_main());
        buf.set_last_in_chain(true);

        let mut out = ngx_chain_t {
            buf: buf.as_ngx_buf_mut(),
            next: ptr::null_mut(),
        };
        return request.output_filter(&mut out);
    }
);
