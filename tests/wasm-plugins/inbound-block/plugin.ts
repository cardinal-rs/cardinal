@external("env", "get_header")
declare function host_get_header(
  namePtr: i32,
  nameLen: i32,
  outPtr: i32,
  outCap: i32
): i32;

@external("env", "get_query_param")
declare function host_get_query_param(
  keyPtr: i32,
  keyLen: i32,
  outPtr: i32,
  outCap: i32
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

function getHeader(name: string): string | null {
  return readIntoString(host_get_header, name, 256);
}

function getQueryParam(name: string): string | null {
  return readIntoString(host_get_query_param, name, 256);
}

export function handle(_ptr: i32, _len: i32): i32 {
  const allowHeader = getHeader("x-allow");
  if (allowHeader != null && allowHeader.toLowerCase() == "true") {
    return 1;
  }

  const allowParam = getQueryParam("allow");
  if (allowParam != null && allowParam.toLowerCase() == "true") {
    return 1;
  }

  return 0;
}

export function __new(size: i32, _align: i32): i32 {
  const buf = new ArrayBuffer(size);
  return changetype<i32>(buf);
}
