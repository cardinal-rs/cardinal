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

function utf8ptr(s: string): i32 {
    return changetype<i32>(String.UTF8.encode(s, false));
}

function utf8len(s: string): i32 {
    return String.UTF8.byteLength(s, false) as i32;
}

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
    return String.UTF8.decodeUnsafe(changetype<usize>(out), n as usize, true);
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

export function handle(_ptr: i32, _len: i32): i32 {
    const tenant = getQueryParam("tenant");
    if (tenant == null || tenant.length == 0) {
        setStatus(422);
        setHeader("x-error", "missing tenant");
        return 0;
    }

    setHeader("x-tenant", tenant);
    setStatus(200);
    return 1;
}

export function alloc(size: i32): i32 {
    const buf = new ArrayBuffer(size);
    return changetype<i32>(buf);
}
