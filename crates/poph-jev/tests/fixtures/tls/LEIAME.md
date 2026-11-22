# Certificados de teste do TLS (HR-10b)

Não são segredo e não servem para nada fora dos testes: são a autoridade certificadora (CA) falsa e
os dois certificados de servidor que os testes usam para provar https de verdade, sem rede externa.

| Arquivo | O que é |
|---|---|
| `ca.der` | CA de teste (autoassinada, `CA:TRUE`, válida por 10 anos). É a raiz extra que o teste acrescenta ao cliente. |
| `servidor-ip.der` | Certificado de servidor com `subjectAltName = IP:127.0.0.1`, assinado pela CA. Usado no teste de https que deve funcionar. |
| `servidor-nome.der` | Certificado de servidor com `subjectAltName = DNS:localhost`, assinado pela CA. Usado no teste de nome errado: o cliente conecta em `127.0.0.1` e recusa. |
| `servidor-chave.der` | Chave privada (PKCS#8 DER) dos dois certificados de servidor. |

Como foram gerados (OpenSSL 3, fora do repositório):

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
