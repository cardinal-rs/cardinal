@external("env", "set_req_var")
declare function host_set_req_var(
  keyPtr: i32,
  keyLen: i32,
  valPtr: i32,
  valLen: i32
): void;

function utf8ptr(s: string): i32 {
  return changetype<i32>(String.UTF8.encode(s, false));
}

function utf8len(s: string): i32 {
  return String.UTF8.byteLength(s, false) as i32;
}

export function handle(_ptr: i32, _len: i32): i32 {
  const key = "shared-token";
  const value = "alpha";
  host_set_req_var(utf8ptr(key), utf8len(key), utf8ptr(value), utf8len(value));
  return 1;
}

export function __new(size: i32, _id: i32): i32 {
  const buf = new ArrayBuffer(size);
  return changetype<i32>(buf);
}
