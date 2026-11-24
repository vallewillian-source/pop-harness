# TLS test certificates (HR-10b)

These are not secrets and have no use outside the tests. They are a fake certificate authority (CA) and
the two server certificates the tests use to prove real https without any external network.

| File | What it is |
|---|---|
| `ca.der` | Test CA (self-signed, `CA:TRUE`, valid for 10 years). It is the extra root the test adds to the client. |
| `servidor-ip.der` | Server certificate with `subjectAltName = IP:127.0.0.1`, signed by the CA. Used in the https test that must succeed. |
| `servidor-nome.der` | Server certificate with `subjectAltName = DNS:localhost`, signed by the CA. Used in the wrong-name test: the client connects to `127.0.0.1` and refuses. |
| `servidor-chave.der` | Private key (PKCS#8 DER) of the two server certificates. |

How they were generated (OpenSSL 3, outside the repository):

```sh
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -keyout ca.key -out ca.pem -days 3650 \
  -subj "/CN=POP Harness Teste CA" \
  -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign"
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:prime256v1 -out srv.key
openssl req -new -key srv.key -out srv-ip.csr -subj "/CN=127.0.0.1"
openssl x509 -req -in srv-ip.csr -CA ca.pem -CAkey ca.key -CAcreateserial -days 3650 -out srv-ip.pem \
  -extfile <(printf "subjectAltName=IP:127.0.0.1\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\n")
openssl req -new -key srv.key -out srv-nome.csr -subj "/CN=localhost"
openssl x509 -req -in srv-nome.csr -CA ca.pem -CAkey ca.key -CAcreateserial -days 3650 -out srv-nome.pem \
  -extfile <(printf "subjectAltName=DNS:localhost\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\n")
for f in ca srv-ip srv-nome; do openssl x509 -in $f.pem -outform DER -out $f.der; done
openssl pkcs8 -topk8 -nocrypt -in srv.key -outform DER -out srv-chave.der
```
