@external("env", "get_header")
declare function host_get_header(
  namePtr: i32,
  nameLen: i32,
  outPtr: i32,
  outCap: i32
): i32;

@external("env", "set_header")
declare function host_set_header(
  setType: i32,
  namePtr: i32,
  nameLen: i32,
  valuePtr: i32,
  valueLen: i32
): void;

@external("env", "set_status")
declare function host_set_status(code: i32): void;

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
  const keyPtr = utf8ptr(key);
  const keyLen = utf8len(key);
  const written = call(keyPtr, keyLen, changetype<i32>(out), outCap);
  if (written < 0) return null;
  return String.UTF8.decodeUnsafe(changetype<usize>(out), written as usize, true);
}

function getHeader(name: string): string | null {
  return readIntoString(host_get_header, name, 256);
}

function setHeader(name: string, value: string): void {
  const nptr = utf8ptr(name);
  const nlen = utf8len(name);
  const vptr = utf8ptr(value);
  const vlen = utf8len(value);
  host_set_header(1, nptr, nlen, vptr, vlen);
}

export function handle(_ptr: i32, _len: i32): i32 {
  const trigger = getHeader("x-set-response");
  if (trigger != null && trigger.toLowerCase() == "true") {
    setHeader("x-wasm-response", "enabled");
    host_set_status(201);
    return 1;
  }

  host_set_status(200);
  return 1;
}

export function __new(size: i32, _align: i32): i32 {
  const buf = new ArrayBuffer(size);
  return changetype<i32>(buf);
}
