//go:build linux

package main

// Minimal proto3 encoding of the Kata 3.29.0 agent requests this lab sends
// (src/libs/protocols/protos/{agent,oci,health}.proto). Field numbers are
// checked against Kata's generated Go types by the lab's provenance test.

import (
	"errors"
	"fmt"
)

func appendVarint(b []byte, v uint64) []byte {
	for v >= 0x80 {
		b = append(b, byte(v)|0x80)
		v >>= 7
	}
	return append(b, byte(v))
}

func appendKey(b []byte, field, wire int) []byte {
	return appendVarint(b, uint64(field)<<3|uint64(wire))
}

func pbString(b []byte, field int, s string) []byte {
	if s == "" {
		return b
	}
	return pbBytes(b, field, []byte(s))
}

// pbBytes always emits the field; use it for present embedded messages.
func pbBytes(b []byte, field int, v []byte) []byte {
	b = appendKey(b, field, 2)
	b = appendVarint(b, uint64(len(v)))
	return append(b, v...)
}

func pbStrings(b []byte, field int, values []string) []byte {
	for _, v := range values {
		b = pbBytes(b, field, []byte(v))
	}
	return b
}

func pbUint(b []byte, field int, v uint64) []byte {
	if v == 0 {
		return b
	}
	return appendVarint(appendKey(b, field, 0), v)
}

func pbInt(b []byte, field int, v int64) []byte { return pbUint(b, field, uint64(v)) }

type pbField struct {
	wire  int
	value uint64
	bytes []byte
}

// pbParse decodes varint and length-delimited fields; other wire types fail.
func pbParse(data []byte) (map[int][]pbField, error) {
	out := map[int][]pbField{}
	for i := 0; i < len(data); {
		key, n := readVarint(data[i:])
		if n <= 0 {
			return nil, errors.New("truncated protobuf key")
		}
		i += n
		field, wire := int(key>>3), int(key&7)
		if field <= 0 {
			return nil, errors.New("invalid protobuf field number")
		}
		switch wire {
		case 0:
			v, n := readVarint(data[i:])
			if n <= 0 {
				return nil, errors.New("truncated protobuf varint")
			}
			i += n
			out[field] = append(out[field], pbField{wire: 0, value: v})
		case 2:
			size, n := readVarint(data[i:])
			if n <= 0 || size > uint64(len(data)-i-n) {
				return nil, errors.New("truncated protobuf bytes")
			}
			i += n
			out[field] = append(out[field], pbField{wire: 2, bytes: data[i : i+int(size)]})
			i += int(size)
		default:
			return nil, fmt.Errorf("unsupported protobuf wire type %d", wire)
		}
	}
	return out, nil
}

func readVarint(b []byte) (uint64, int) {
	var v uint64
	for i := 0; i < len(b) && i < 10; i++ {
		if i == 9 && b[i] > 1 {
			return 0, 0
		}
		v |= uint64(b[i]&0x7f) << (7 * i)
		if b[i] < 0x80 {
			return v, i + 1
		}
	}
	return 0, 0
}

// ttrpc Request{service=1, method=2, payload=3, timeout_nano=4}.
func ttrpcRequest(service, method string, payload []byte, timeoutNano int64) []byte {
	b := pbString(nil, 1, service)
	b = pbString(b, 2, method)
	if len(payload) > 0 {
		b = pbBytes(b, 3, payload)
	}
	return pbInt(b, 4, timeoutNano)
}

// ttrpc Response{status=1 google.rpc.Status{code=1, message=2}, payload=2}.
func ttrpcResponse(data []byte) (code int64, message string, payload []byte, err error) {
	fields, err := pbParse(data)
	if err != nil {
		return 0, "", nil, err
	}
	if s := fields[1]; len(s) > 0 {
		if s[len(s)-1].wire != 2 {
			return 0, "", nil, errors.New("invalid RPC status wire type")
		}
		status, err := pbParse(s[len(s)-1].bytes)
		if err != nil {
			return 0, "", nil, err
		}
		if c := status[1]; len(c) > 0 {
			if c[len(c)-1].wire != 0 {
				return 0, "", nil, errors.New("invalid RPC code wire type")
			}
			code = int64(int32(c[len(c)-1].value))
		}
		if m := status[2]; len(m) > 0 {
			if m[len(m)-1].wire != 2 {
				return 0, "", nil, errors.New("invalid RPC message wire type")
			}
			message = string(m[len(m)-1].bytes)
		}
	}
	if p := fields[2]; len(p) > 0 {
		if p[len(p)-1].wire != 2 {
			return 0, "", nil, errors.New("invalid RPC payload wire type")
		}
		payload = p[len(p)-1].bytes
	}
	return code, message, payload, nil
}

func lastUint(fields map[int][]pbField, field int) uint64 {
	if f := fields[field]; len(f) > 0 {
		return f[len(f)-1].value
	}
	return 0
}

// CreateSandboxRequest{hostname=1, sandbox_id=5}.
func createSandboxRequest(hostname, sandboxID string) []byte {
	return pbString(pbString(nil, 1, hostname), 5, sandboxID)
}

type mountSpec struct {
	destination, kind, source string
	options                   []string
}

type syntheticSpec struct {
	rootfs      string
	cgroupsPath string
	args        []string
	env         []string
	mounts      []mountSpec
	memoryLimit int64
	cpuQuota    int64
	cpuPeriod   uint64
}

var syntheticCaps = []string{"CAP_CHOWN", "CAP_DAC_OVERRIDE", "CAP_FSETID", "CAP_FOWNER",
	"CAP_MKNOD", "CAP_NET_RAW", "CAP_SETGID", "CAP_SETUID", "CAP_SETFCAP", "CAP_SETPCAP",
	"CAP_NET_BIND_SERVICE", "CAP_SYS_CHROOT", "CAP_KILL", "CAP_AUDIT_WRITE"}

// ociSpec encodes Spec{Version=1, Process=2, Root=3, Hostname=4, Mounts=5, Linux=8}.
func ociSpec(s syntheticSpec) []byte {
	caps := pbStrings(nil, 1, syntheticCaps) // Bounding
	caps = pbStrings(caps, 2, syntheticCaps) // Effective
	caps = pbStrings(caps, 4, syntheticCaps) // Permitted
	process := pbBytes(nil, 3, nil)          // User{} (uid/gid 0)
	process = pbStrings(process, 4, s.args)  // Args
	process = pbStrings(process, 5, s.env)   // Env
	process = pbString(process, 6, "/")      // Cwd
	process = pbBytes(process, 7, caps)      // Capabilities
	root := pbString(nil, 1, s.rootfs)       // Root.Path
	spec := pbString(nil, 1, "1.0.2")
	spec = pbBytes(spec, 2, process)
	spec = pbBytes(spec, 3, root)
	spec = pbString(spec, 4, "finlab")
	for _, m := range s.mounts {
		mount := pbString(nil, 1, m.destination)
		mount = pbString(mount, 2, m.source)
		mount = pbString(mount, 3, m.kind)
		mount = pbStrings(mount, 4, m.options)
		spec = pbBytes(spec, 5, mount)
	}
	memory := pbInt(nil, 1, s.memoryLimit)
	cpu := pbInt(nil, 2, s.cpuQuota)
	cpu = pbUint(cpu, 3, s.cpuPeriod)
	resources := pbBytes(nil, 2, memory)
	resources = pbBytes(resources, 3, cpu)
	linux := pbBytes(nil, 4, resources)
	linux = pbString(linux, 5, s.cgroupsPath)
	for _, ns := range []string{"pid", "mount", "ipc", "uts"} {
		linux = pbBytes(linux, 6, pbString(nil, 1, ns))
	}
	return pbBytes(spec, 8, linux)
}

// CreateContainerRequest{container_id=1, exec_id=2, OCI=6}.
func createContainerRequest(cid, execID string, spec []byte) []byte {
	return pbBytes(pbString(pbString(nil, 1, cid), 2, execID), 6, spec)
}

func containerRequest(cid string) []byte { return pbString(nil, 1, cid) }

// SignalProcessRequest{container_id=1, exec_id=2, signal=3}.
func signalProcessRequest(cid, execID string, signal uint32) []byte {
	return pbUint(pbString(pbString(nil, 1, cid), 2, execID), 3, uint64(signal))
}

// WaitProcessRequest{container_id=1, exec_id=2}.
func waitProcessRequest(cid, execID string) []byte {
	return pbString(pbString(nil, 1, cid), 2, execID)
}
