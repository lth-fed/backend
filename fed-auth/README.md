YOU MAY NOT HAVE SEVERAL INSTANCES OF THIS BECAUSE OF THE SAML REQUEST AUTH ID CACHE.

Use the following to generate the required private key: `openssl genpkey -algorithm ed25519 -outform der | base64`.

Use the following to generate the required SAML keys:
<!-- $ openssl req -x509 -newkey rsa:4096 -keyout /tmp/pk.pem -out /tmp/cert.pem -days 7300 -nodes -subj "/C=SE/ST=Sverige/L=Lund/O=Maintainers of Teknologappen/CN=teknologappen.se" 2>/dev/null-->
<!-- $ openssl genrsa -out /tmp/pk.pem 4096 -->
<!-- $ openssl rsa -in /tmp/pk.pem -pubout -out /tmp/pub.pem -->
```bash
<!-- $ openssl req -x509 -newkey rsa:4096 -keyout /tmp/pk.pem -out /tmp/cert.pem -days 7300 -nodes -subj "/C=SE/ST=Sverige/L=Lund/O=Maintainers of Teknologappen/CN=teknologappen.se" 2>/dev/null-->
$ echo "Privata nyckeln:"
$ cat /tmp/pk.pem | base64 -w0
$ echo -e "\nCertifikatet:"
$ cat /tmp/cert.pem | base64 -w0
$ rm -f /tmp/pk.pem /tmp/cert.pem
```
