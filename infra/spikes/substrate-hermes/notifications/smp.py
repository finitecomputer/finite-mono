"""Spike-only SMP v14 subset. Uses upstream's existing notification capability.

No message decoding, no TLS verification bypass: pin the queue's CA and verify
its leaf. v14 is intentionally narrow; replace this client with upstream library
integration before production. Never log frames or credentials.
"""
import base64, hashlib, os, socket, ssl, struct
from cryptography import x509
from cryptography.hazmat.primitives import serialization as ser
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey

def b64(b): return base64.urlsafe_b64encode(b).decode()
def un64(s): return base64.urlsafe_b64decode(s)
def short(b):
    if len(b)>255: raise ValueError('oversized field')
    return bytes([len(b)])+b

def take(b):
    n=b[0]
    if len(b)<n+1: raise ValueError('truncated field')
    return b[1:n+1],b[n+1:]

def der_private(k): return k.private_bytes(ser.Encoding.DER,ser.PrivateFormat.PKCS8,ser.NoEncryption())
def der_public(k): return k.public_key().public_bytes(ser.Encoding.DER,ser.PublicFormat.SubjectPublicKeyInfo)

class SMP:
    def __init__(self,host,port,key_hash):
        host=next(h for h in host.split(',') if not h.endswith('.onion'))
        ctx=ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        ctx.check_hostname=False; ctx.verify_mode=ssl.CERT_NONE
        # SimpleX uses a private pinned CA rather than public Web PKI. Validate
        # that chain explicitly below, before sending any signed commands.
        ctx.minimum_version=ctx.maximum_version=ssl.TLSVersion.TLSv1_2
        # Upstream explicitly disables TLS secure renegotiation; permit the
        # initial handshake only. No renegotiation is used by this client.
        ctx.options |= 0x4  # OpenSSL SSL_OP_LEGACY_SERVER_CONNECT
        ctx.set_ecdh_curve("X25519")
        self.sock=ctx.wrap_socket(socket.create_connection((host,int(port or 5223)),timeout=20),server_hostname=host)
        chain=self.sock._sslobj.get_unverified_chain()
        certs=[x509.load_pem_x509_certificate(c.public_bytes().encode()) for c in chain]
        expected=un64(key_hash) if isinstance(key_hash,(str,bytes)) and len(key_hash)==44 else key_hash
        if hashlib.sha256(certs[-1].public_bytes(ser.Encoding.DER)).digest()!=expected:
            self.close(); raise ValueError('SMP CA fingerprint mismatch')
        for child,parent in zip(certs,certs[1:]): child.verify_directly_issued_by(parent)
        certs[-1].verify_directly_issued_by(certs[-1])
        from datetime import datetime, timezone
        now=datetime.now(timezone.utc)
        if any(not c.not_valid_before_utc <= now <= c.not_valid_after_utc for c in certs):
            self.close(); raise ValueError('SMP certificate expired')
        hs=self.block(); low,high=struct.unpack('!HH',hs[:4]); self.session,_=take(hs[4:])
        if not low<=14<=high: raise ValueError(f'server range {low}..{high} does not support SMP v14')
        if self.session!=self.sock.get_channel_binding('tls-unique'): raise ValueError('TLS session mismatch')
        self.put(struct.pack('!H',14)+short(expected)+short(der_public(X25519PrivateKey.generate()))+b"T")
        self.pending=[]
    def close(self): self.sock.close()
    def block(self):
        data=b''
        while len(data)<16384:
            part=self.sock.recv(16384-len(data))
            if not part: raise EOFError('SMP disconnected')
            data+=part
        n=struct.unpack('!H',data[:2])[0]
        if n>16382: raise ValueError('invalid block')
        return data[2:2+n]
    def put(self,data):
        if len(data)>16382: raise ValueError('oversized block')
        self.sock.sendall(struct.pack('!H',len(data))+data+b'#'*(16382-len(data)))
    def send(self,entity,cmd,key=None):
        corr=os.urandom(24)
        frame=short(self.session)+short(corr)+short(entity)+cmd
        sig=key.sign(frame) if key else b''
        wire=short(sig)+frame[len(short(self.session)):]
        self.put(b'\x01'+struct.pack('!H',len(wire))+wire)
        return corr
    def receive(self):
        if self.pending: return self.pending.pop(0)
        data=self.block(); count=data[0]; data=data[1:]
        for _ in range(count):
            n=struct.unpack('!H',data[:2])[0]; frame=data[2:2+n]; data=data[2+n:]
            _,frame=take(frame)
            corr,frame=take(frame); entity,cmd=take(frame)
            self.pending.append((corr,entity,cmd))
        return self.pending.pop(0)
    def command(self,entity,cmd,key=None):
        corr=self.send(entity,cmd,key)
        while True:
            rc,entity,result=self.receive()
            if rc==corr: return result
            raise RuntimeError('unexpected unsolicited event during enrollment')
