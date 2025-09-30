@external("env", "host_signal")
declare function host_signal(ptr: i32, len: i32): i32;

@external("env", "set_header")
declare function host_set_header(
  namePtr: i32,
  nameLen: i32,
  valPtr: i32,
  valLen: i32
): void;

@external("env", "set_status")
declare function host_set_status(code: i32): void;

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
  const buf = new ArrayBuffer(4);
  const ptr = changetype<i32>(buf);
  const len = buf.byteLength;

  host_signal(ptr, len);

  const bytes = Uint8Array.wrap(buf);
  const first = bytes[0].toString();
  writeHeader("x-host-memory", first);
  writeHeader("x-host-signal", "called");
  host_set_status(200);
  return 1;
}

export function __new(size: i32, _id: i32): i32 {
  const buf = new ArrayBuffer(size);
  return changetype<i32>(buf);
}
