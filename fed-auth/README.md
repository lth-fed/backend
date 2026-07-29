YOU MAY NOT HAVE SEVERAL INSTANCES OF THIS BECAUSE OF THE SAML REQUEST AUTH ID CACHE.

You may need to run `createdb -h localhost -p 5432 -U postgres auth` when first starting this.

You need to have `xmlsec1` installed to run this.

Use the following to generate the required private key: `openssl genpkey -algorithm ed25519 -outform der | base64`.

Use the following to generate the required SAML keys:
<!-- $ openssl req -x509 -newkey rsa:4096 -keyout /tmp/pk.pem -out /tmp/cert.pem -days 7300 -nodes -subj "/C=SE/ST=Sverige/L=Lund/O=Maintainers of Teknologappen/CN=teknologappen.se" 2>/dev/null-->
<!-- $ openssl genrsa -out /tmp/pk.pem 4096 -->
<!-- $ openssl rsa -in /tmp/pk.pem -pubout -out /tmp/pub.pem -->
```bash
$ openssl req -x509 -newkey rsa:4096 -keyout /tmp/pk.pem -out /tmp/cert.pem -days 7300 -nodes -subj "/C=SE/ST=Sverige/L=Lund/O=Maintainers of Teknologappen/CN=teknologappen.se" 2>/dev/null
$ echo "Privata nyckeln:"
$ cat /tmp/pk.pem | base64 -w0
$ echo -e "\nCertifikatet:"
$ cat /tmp/cert.pem | base64 -w0
$ rm -f /tmp/pk.pem /tmp/cert.pem
```

# Test the OICD endpoints

```
http://localhost:8001/oidc/v1/authorize?client_id=teknologappen&redirect_uri=http%3A%2F%2Flocalhost%3A8000&response_type=code&scope=openid&state=d8b16b2b-7270-481b-ad4d-460cf36cde7f&code_challenge=0eEMCayyIHqsvVRCbxGQ_Q1JnuxkIaiInrC7fNndDZg&code_challenge_method=S256&providers=test

http://localhost:8001/oidc/v1/token?code=<code here>&code_verifier=mCDrfQLIngfAIJo4tr54iKLJKpgWM-jsjX3VGa8YV0U&grant_type=authorization_code&client_id=teknologappen&redirect_uri=http://localhost:8000
```

or add the following to the end of the swagger UI (OpenAPI preview) url:

```
&scope=openid&code_challenge=0eEMCayyIHqsvVRCbxGQ_Q1JnuxkIaiInrC7fNndDZg&code_challenge_method=S256&providers=test
```

or with provider selection:

```
http://localhost:8001/oidc/v1/authorize?client_id=teknologappen&redirect_uri=http%3A%2F%2Flocalhost%3A8000&response_type=code&scope=openid&state=d8b16b2b-7270-481b-ad4d-460cf36cde7f&code_challenge=0eEMCayyIHqsvVRCbxGQ_Q1JnuxkIaiInrC7fNndDZg&code_challenge_method=S256
````
