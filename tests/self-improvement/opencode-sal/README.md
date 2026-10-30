# Pruebas del circuito OpenCode Sal (T92–T93)

- **Paridad (T92):** comparar aceptadas en R41–R50 entre `soso-improve campana` (Rust) y `soso-improve-sal campana DIR` con el mismo modelo servido por soso.
- **Autonomía (T93):** tras promover base, `opencode reinstalar` recompila con `/bin/sal` y `soso-pkg instalar` el paquete generado; rollback si el validador falla.

Las pruebas automatizadas de HTTP mockean el servidor (`opencode-sal/tests/mock_http_server.py`); no llaman al modelo real.
