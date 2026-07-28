# Seed databases

- run `cargo run --bin seed-dev` in `minilith`
- run `./test-data/import.sh ./test-data/auth.sql` in this directory

# Get ticket

```bash
curl -v -X PUT http://localhost:8000/v0/tickets/queue \
    -H "authorization: Bearer test:ma5657ed-s" \
    -H "content-type: application/json" \
    -d '{"ticket_kind": "00000000-0000-0000-0000-0000a0000002"}'
```

# Check status

```bash
curl -v -X GET http://localhost:8000/v0/tickets/queue \
    -H "authorization: Bearer test:ma5657ed-s"
```

# Buy the ticket

```bash
curl -v -X POST http://localhost:8000/v0/tickets/reservation/buy \
    -H "authorization: Bearer test:ma5657ed-s" \
    -H "content-type: application/json" \
    -d '{
        "ticket_kind": "00000000-0000-0000-0000-0000a0000002",
        "provider": "swish",
        "addons": []
    }'
```

# Check status

Same as above.

# Wait 4s for the swish to send a paid callback (do it manually if local)

```bash
curl -v -X POST http://localhost:8002/v0/swish-callback \
    -H "callbackIdentifier: <the callback identifier from the DB>" \
    -H "content-type: application/json" \
    -d '{
        "id": "<txn id from DB>",
        "paymentReference": "hejsan hoppsan :)",
        "status": "PAID"
    }'
```

# Check status

Same as above. If not found, you've gotten the ticket! Check my tickets using:

```bash
curl -v -X GET http://localhost:8000/v0/tickets \
    -H "authorization: Bearer test:ma5657ed-s"
```
