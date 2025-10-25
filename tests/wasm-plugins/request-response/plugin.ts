@external("env", "set_header")
declare function host_set_header(
  setType: i32,
  namePtr: i32,
  nameLen: i32,
  valuePtr: i32,
  valueLen: i32
): void;

function utf8ptr(value: string): i32 {
  return changetype<i32>(String.UTF8.encode(value, false));
}

function utf8len(value: string): i32 {
  return String.UTF8.byteLength(value, false) as i32;
}

function setRequestHeader(name: string, value: string): void {
  host_set_header(0, utf8ptr(name), utf8len(name), utf8ptr(value), utf8len(value));
}

function setResponseHeader(name: string, value: string): void {
  host_set_header(1, utf8ptr(name), utf8len(name), utf8ptr(value), utf8len(value));
}

export function handle(_ptr: i32, _len: i32): i32 {
  setRequestHeader("User-Id", "beta");
  setResponseHeader("x-plugin-applied", "true");
  return 1;
}

export function alloc(size: i32): i32 {
  const buffer = new ArrayBuffer(size);
  return changetype<i32>(buffer);
}
