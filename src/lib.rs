use serde::{Deserialize, Serialize};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

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
    fn http_request_with_headers(
        url_ptr: *const u8,
        url_len: i32,
        method_ptr: *const u8,
        method_len: i32,
        headers_ptr: *const u8,
        headers_len: i32,
        body_ptr: *const u8,
        body_len: i32,
    ) -> i32;
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

fn fetch_url_post(url: &str, post_body: &str) -> Result<Vec<u8>, String> {
    let method = "POST";
    let headers_json = r#"{"Content-Type":"application/x-www-form-urlencoded"}"#;

    let handle = unsafe {
        http_request_with_headers(
            url.as_ptr(),
            url.len() as i32,
            method.as_ptr(),
            method.len() as i32,
            headers_json.as_ptr(),
            headers_json.len() as i32,
            post_body.as_ptr(),
            post_body.len() as i32,
        )
    };
    if handle < 0 {
        return Err(format!("HTTP POST request failed: {}", -handle));
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
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default = "default_page")]
    page: u32,
    #[serde(default)]
    author: Option<String>,
    #[serde(default, rename = "narrator")]
    _narrator: Option<String>,
}

impl SearchParams {
    fn keyword(&self) -> Result<&str, String> {
        if let Some(title) = self
            .title
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Ok(title);
        }

        if let Some(query) = self
            .query
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Ok(query);
        }

        Err("Missing required search field: title".to_string())
    }
}

fn default_page() -> u32 {
    1
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

    // Try primary API first
    match try_primary_search(&params) {
        Ok(result) => Ok(result),
        Err(primary_err) => {
            // If primary fails, try backup API
            match try_backup_search(&params) {
                Ok(result) => Ok(result),
                Err(backup_err) => Err(format!(
                    "Both APIs failed. Primary: {}; Backup: {}",
                    primary_err, backup_err
                )),
            }
        }
    }
}

fn try_primary_search(params: &SearchParams) -> Result<SearchResult, String> {
    let keyword = params.keyword()?;
    let url = format!(
        "https://m.ypshuo.com/api/novel/search?keyword={}&searchType=1&page={}",
        url::form_urlencoded::byte_serialize(keyword.as_bytes()).collect::<String>(),
        params.page
    );

    let body = fetch_url(&url)?;
    let resp: YpshuoResponse<YpshuoSearchData> =
        serde_json::from_slice(&body).map_err(|e| e.to_string())?;

    if resp.code != "00" {
        return Err(format!("API Error: {}", resp.code));
    }

    let book_list = match resp.data {
        Some(d) => d.data,
        None => Vec::new(),
    };

    let mut items: Vec<BookItem> = book_list
        .into_iter()
        .map(|b| {
            let clean_title = b
                .novel_name
                .split('丨')
                .next()
                .unwrap_or(&b.novel_name)
                .split('|')
                .next()
                .unwrap_or(&b.novel_name)
                .trim()
                .to_string();

            let mut cover = b.novel_img;
            if cover.starts_with("//") {
                cover = format!("https:{}", cover);
            } else if cover.starts_with("http://") {
                cover = cover.replacen("http://", "https://", 1);
            }

            BookItem {
                id: b.id.to_string(),
                title: clean_title,
                author: b.author_name,
                cover_url: Some(cover),
                intro: Some(b.synopsis),
                narrator: None,
                tags: b
                    .tags
                    .unwrap_or_default()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
                chapter_count: None,
                duration: None,
            }
        })
        .collect();

    apply_author_filter(&mut items, &params.author);

    Ok(SearchResult {
        items,
        total: 100,
        page: params.page,
        page_size: 20,
    })
}

fn try_backup_search(params: &SearchParams) -> Result<SearchResult, String> {
    let keyword = params.keyword()?;
    let url = "https://www.youshu.me/modules/article/search.php";

    // Build POST body
    let post_body = format!(
        "searchtype=all&searchkey={}&t_btnsearch=",
        url::form_urlencoded::byte_serialize(keyword.as_bytes()).collect::<String>()
    );

    let body = fetch_url_post(url, &post_body)?;
    let html = String::from_utf8_lossy(&body);

    // Parse HTML to extract book information
    let items = parse_youshu_html(&html)?;

    let mut filtered_items = items;
    apply_author_filter(&mut filtered_items, &params.author);

    Ok(SearchResult {
        items: filtered_items,
        total: 100,
        page: params.page,
        page_size: 20,
    })
}

fn parse_youshu_html(html: &str) -> Result<Vec<BookItem>, String> {
    // Check if this is a book detail page (redirect when only one result)
    if html.contains("<div class=\"divbox cf blockn\">") && !html.contains("<div class=\"c_row\">")
    {
        // This is a book detail page, parse it as a single book
        if let Some(book) = parse_book_detail_page(html) {
            return Ok(vec![book]);
        }
        return Err("Failed to parse book detail page".to_string());
    }

    // Otherwise, parse as search results list
    let mut items = Vec::new();

    // Split by book entries (each c_row div contains one book)
    let parts: Vec<&str> = html.split("<div class=\"c_row\">").collect();

    for part in parts.iter().skip(1) {
        if let Some(book) = parse_single_book(part) {
            items.push(book);
        }
    }

    if items.is_empty() {
        return Err("No books found in HTML response".to_string());
    }

    Ok(items)
}

fn parse_single_book(html: &str) -> Option<BookItem> {
    // Extract book ID from URL like /book/293282
    let id = extract_between(html, "/book/", "\"")?;

    // Extract title - it's inside <span class="c_subject"><a href="/book/ID"><span class="hot">TITLE</span></a></span>
    // or <span class="c_subject"><a href="/book/ID">TITLE</a></span>
    let title_section = extract_between(html, "<span class=\"c_subject\">", "</span>")?;
    let title = if let Some(hot_title) =
        extract_between(title_section, "<span class=\"hot\">", "</span>")
    {
        hot_title
    } else {
        // No <span class="hot">, extract from <a href="/book/ID">TITLE</a>
        extract_between(title_section, "\">", "</a>")?
    };

    // Extract author
    let author_section = extract_between(html, "<span class=\"c_label\">作者：</span>", "</span>")?;
    let author = extract_between(author_section, "<span class=\"c_value\">", "")?.to_string();

    // Extract cover URL - look specifically for the book cover image with width:80px style
    let cover_url = extract_between(html, "<img src=\"", "\" style=\"width:80px").map(|s| {
        let mut url = s.to_string();
        if url.starts_with("//") {
            url = format!("https:{}", url);
        } else if url.starts_with("http://") {
            url = url.replace("http://", "https://");
        }
        url
    });

    // Extract intro/description
    let intro = extract_between(html, "<div class=\"c_description\">", "</div>")
        .map(|s| s.trim().to_string());

    // Extract tags
    let tags = if let Some(tag_section) =
        extract_between(html, "<span class=\"c_label\">标签：</span>", "</span>")
    {
        if let Some(tag_value) = extract_between(tag_section, "<span class=\"c_value\">", "") {
            tag_value
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    Some(BookItem {
        id: id.to_string(),
        title: decode_html_entities(&title),
        author: decode_html_entities(&author),
        cover_url,
        intro: intro.map(|s| decode_html_entities(&s)),
        narrator: None,
        tags,
        chapter_count: None,
        duration: None,
    })
}

fn parse_book_detail_page(html: &str) -> Option<BookItem> {
    // Extract book ID from URL in the page (e.g., in links like /book/20486)
    let id = extract_between(html, "jumpurl=%2Fbook%2F", "&")?;

    // Extract title - it's in a span with font-size:20px
    let title = extract_between(
        html,
        "font-size:20px;font-weight:bold;color:#f27622;\">",
        "</span>",
    )?;

    // Extract author - it's in the link text
    // HTML: <span>&nbsp;&nbsp;作者：<a href="..." target="_blank">西风紧</a></span>
    let author_link = extract_between(
        html,
        "作者：<a href=\"/modules/article/authorarticle.php?author=",
        "</a>",
    )?;
    let author = author_link.rsplit('>').next()?.trim();

    // Extract cover URL from the img src attribute
    // HTML: <a href="..." class="book-detail-img" target="_blank"><img src="http://..." style="border:1px...
    let cover_url = extract_between(
        html,
        "class=\"book-detail-img\" target=\"_blank\"><img src=\"",
        "\" style=\"border:1px",
    )
    .map(|s| {
        let mut url = s.to_string();
        if url.starts_with("//") {
            url = format!("https:{}", url);
        } else if url.starts_with("http://") {
            url = url.replacen("http://", "https://", 1);
        }
        url
    });

    // Extract intro/description from tabvalue
    let intro = extract_between(
        html,
        "<div class=\"tabvalue\" style=\"height:180px;\">",
        "</div>",
    )
    .and_then(|s| extract_between(s, "overflow-y:scroll;\">", ""))
    .map(|s| s.trim().to_string());

    // Extract tags
    let tags = if let Some(tag_section) = extract_between(html, "<b>标签：</b>", "</div>") {
        let mut tag_list = Vec::new();
        let tag_parts: Vec<&str> = tag_section.split("<a class=\"tag-link\"").collect();
        for part in tag_parts.iter().skip(1) {
            if let Some(tag_text) = extract_between(part, "\">", "</a>") {
                tag_list.push(tag_text.trim().to_string());
            }
        }
        tag_list
    } else {
        Vec::new()
    };

    Some(BookItem {
        id: id.to_string(),
        title: decode_html_entities(&title),
        author: decode_html_entities(&author),
        cover_url,
        intro: intro.map(|s| decode_html_entities(&s)),
        narrator: None,
        tags,
        chapter_count: None,
        duration: None,
    })
}

fn extract_between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let start_pos = text.find(start)? + start.len();
    let remaining = &text[start_pos..];

    if end.is_empty() {
        return Some(remaining.split('<').next()?.trim());
    }

    let end_pos = remaining.find(end)?;
    Some(remaining[..end_pos].trim())
}

fn decode_html_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

fn apply_author_filter(items: &mut Vec<BookItem>, author_filter: &Option<String>) {
    if let Some(author_filter) = author_filter {
        if !author_filter.is_empty() {
            let normalized_filter = author_filter.trim().to_lowercase();
            let index = items.iter().position(|item| {
                let author = item.author.trim().to_lowercase();
                !author.is_empty()
                    && (author.contains(&normalized_filter) || normalized_filter.contains(&author))
            });

            if let Some(idx) = index {
                let matched_item = items.remove(idx);
                items.insert(0, matched_item);
            }
        }
    }
}
