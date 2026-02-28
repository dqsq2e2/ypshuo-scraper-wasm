use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use serde::{Deserialize, Serialize};

// --- FFI Helpers ---

#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

#[no_mangle]
pub extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    unsafe {
        let _ = Vec::from_raw_parts(ptr, 0, len);
    }
}

// --- Plugin Interface ---

#[no_mangle]
pub extern "C" fn initialize() -> i32 {
    0 // Success
}

#[no_mangle]
pub extern "C" fn shutdown() -> i32 {
    0 // Success
}

#[no_mangle]
pub extern "C" fn invoke(method_ptr: *const c_char, params_ptr: *const c_char) -> *mut c_char {
    let method = unsafe { CStr::from_ptr(method_ptr).to_string_lossy() };
    let params_json = unsafe { CStr::from_ptr(params_ptr).to_string_lossy() };

    let result = match method.as_ref() {
        "search" => handle_search(&params_json).map(|r| serde_json::to_string(&r).unwrap()),
        _ => Err(format!("Unknown method: {}", method)),
    };

    let response_json = match result {
        Ok(json) => json,
        Err(e) => serde_json::json!({ "error": e }).to_string(),
    };

    CString::new(response_json).unwrap().into_raw()
}

// --- Host Functions ---

#[link(wasm_import_module = "ting_env")]
extern "C" {
    fn http_request(url_ptr: *const u8, url_len: i32) -> i32;
    fn http_response_size(handle: i32) -> i32;
    fn http_read_body(handle: i32, ptr: *mut u8, len: i32) -> i32;
}

fn fetch_url(url: &str) -> Result<Vec<u8>, String> {
    let handle = unsafe { http_request(url.as_ptr(), url.len() as i32) };
    if handle < 0 {
        return Err(format!("HTTP request failed: {}", -handle));
    }
    let size = unsafe { http_response_size(handle) };
    if size < 0 {
        return Err("Failed to get response size".to_string());
    }
    let mut body = vec![0u8; size as usize];
    let read_len = unsafe { http_read_body(handle, body.as_mut_ptr(), size) };
    if read_len < 0 {
        return Err("Failed to read body".to_string());
    }
    Ok(body)
}

// --- Handlers ---

#[derive(Deserialize)]
struct SearchParams {
    query: String,
    page: u32,
}

#[derive(Serialize)]
struct SearchResult {
    items: Vec<BookItem>,
    total: u32,
    page: u32,
    page_size: u32,
}

#[derive(Serialize, Deserialize)]
struct BookItem {
    id: String,
    title: String,
    author: String,
    cover_url: Option<String>,
    intro: Option<String>,
    #[serde(default)]
    narrator: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    chapter_count: Option<u32>,
    #[serde(default)]
    duration: Option<u64>,
}

// Ypshuo API Models
#[derive(Deserialize)]
struct YpshuoResponse<T> {
    code: String,
    data: Option<T>, // Data can be null if not found
}

#[derive(Deserialize)]
struct YpshuoSearchData {
    data: Vec<YpshuoBook>,
}

#[derive(Deserialize)]
struct YpshuoBook {
    id: u32,
    novel_name: String,
    // category_id: u32,
    tags: Option<String>,
    author_name: String,
    novel_img: String, // Full URL
    synopsis: String,
    // word_number: u32,
    // update_status: u32,
    // status: u32, // Added
}

// Handler Implementations

fn handle_search(params_json: &str) -> Result<SearchResult, String> {
    let params: SearchParams = serde_json::from_str(params_json).map_err(|e| e.to_string())?;
    
    let url = format!(
        "https://m.ypshuo.com/api/novel/search?keyword={}&searchType=1&page={}",
        url::form_urlencoded::byte_serialize(params.query.as_bytes()).collect::<String>(),
        params.page
    );

    let body = fetch_url(&url)?;
    // Use serde_json::Value to inspect response structure first if needed, but here we adapt the model
    let resp: YpshuoResponse<YpshuoSearchData> = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    
    if resp.code != "00" {
        return Err(format!("API Error: {}", resp.code));
    }

    // Handle case where data is None (e.g. no results)
    let book_list = match resp.data {
        Some(d) => d.data,
        None => Vec::new(),
    };

    let items = book_list.into_iter().map(|b| {
        // 1. Clean title: remove suffix after first "丨" or "|"
        let clean_title = b.novel_name
            .split('丨')
            .next()
            .unwrap_or(&b.novel_name)
            .split('|')
            .next()
            .unwrap_or(&b.novel_name)
            .trim()
            .to_string();

        // 2. Fix cover URL: add https protocol if missing
        let mut cover = b.novel_img;
        if cover.starts_with("//") {
            cover = format!("https:{}", cover);
        }

        BookItem {
            id: b.id.to_string(),
            title: clean_title,
            author: b.author_name,
            cover_url: Some(cover),
            intro: Some(b.synopsis),
            narrator: None,
            tags: b.tags.unwrap_or_default().split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            chapter_count: None,
            duration: None,
        }
    }).collect();

    Ok(SearchResult {
        items,
        total: 100,
        page: params.page,
        page_size: 20,
    })
}

