@external("env", "get_req_var")
declare function host_get_req_var(
  keyPtr: i32,
  keyLen: i32,
  outPtr: i32,
  outCap: i32
): i32;

@external("env", "set_header")
declare function host_set_header(
  namePtr: i32,
  nameLen: i32,
  valuePtr: i32,
  valueLen: i32
): void;

function utf8ptr(s: string): i32 {
  return changetype<i32>(String.UTF8.encode(s, false));
}

function utf8len(s: string): i32 {
  return String.UTF8.byteLength(s, false) as i32;
}

function writeHeader(name: string, value: string): void {
  host_set_header(utf8ptr(name), utf8len(name), utf8ptr(value), utf8len(value));
}

export function handle(_ptr: i32, _len: i32): i32 {
  const out = new ArrayBuffer(256);
  const written = host_get_req_var(
    utf8ptr("shared-token"),
    utf8len("shared-token"),
    changetype<i32>(out),
    256
  );

  if (written > 0) {
    const value = String.UTF8.decodeUnsafe(
      changetype<usize>(out),
      written as usize,
      true
    );
    writeHeader("x-shared-token", value);
  }

  return 1;
}

export function __new(size: i32, _id: i32): i32 {
  const buf = new ArrayBuffer(size);
  return changetype<i32>(buf);
}
