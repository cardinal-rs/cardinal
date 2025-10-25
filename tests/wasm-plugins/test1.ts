@external("env", "get_header")
declare function host_get_header(
    namePtr: i32, nameLen: i32,
    outPtr: i32, outCap: i32
): i32;

@external("env", "set_header")
declare function host_set_header(
    setType: i32,
    namePtr: i32,
    nameLen: i32,
    valPtr: i32,
    valLen: i32
): void;

@external("env", "set_status")
declare function host_set_status(code: i32): void;

@external("env", "get_query_param")
declare function host_get_query_param(
    keyPtr: i32, keyLen: i32,
    outPtr: i32, outCap: i32
): i32;

// ---- UTF-8 helpers (no destructuring) ----
function utf8ptr(s: string): i32 {
    // ArrayBuffer -> pointer
    return changetype<i32>(String.UTF8.encode(s, /*nullTerminated*/ false));
}
function utf8len(s: string): i32 {
    // exact byte length without NUL
    return String.UTF8.byteLength(s, /*nullTerminated*/ false) as i32;
}

// call a host string->string getter into a temp buffer and decode to string|null
function readIntoString(
    call: (keyPtr: i32, keyLen: i32, outPtr: i32, outCap: i32) => i32,
    key: string,
    outCap: i32 = 256
): string | null {
    const out = new ArrayBuffer(outCap);
    const kptr = utf8ptr(key);
    const klen = utf8len(key);
    const n = call(kptr, klen, changetype<i32>(out), outCap);
    if (n < 0) return null;
    // decodeUnsafe(ptr, len, terminate?)
    return String.UTF8.decodeUnsafe(changetype<usize>(out), n as usize, true);
}

function getHeader(name: string, cap: i32 = 256): string | null {
    return readIntoString(host_get_header, name, cap);
}

function getQueryParam(name: string, cap: i32 = 256): string | null {
    return readIntoString(host_get_query_param, name, cap);
}

function setHeader(name: string, value: string): void {
    const nptr = utf8ptr(name);
    const nlen = utf8len(name);
    const vptr = utf8ptr(value);
    const vlen = utf8len(value);
    host_set_header(1, nptr, nlen, vptr, vlen);
}

function setStatus(code: i32): void {
    host_set_status(code);
}

export function handle(ptr: i32, len: i32): i32 {
    const auth = getHeader("authorization");
    if (auth == null) {
        setStatus(401);
        setHeader("X-Authorization-Success", "Failed");
        return 0; // respond
    }

    // use query param too, just to exercise the import
    const user = getQueryParam("user");
    if (user != null) {
        setHeader("x-user", user);
    }

    setHeader("x-plugin", "demo");
    return 1; // continue
}

export function alloc(size: i32): i32 {
    const buf = new ArrayBuffer(size);
    return changetype<i32>(buf);
}
