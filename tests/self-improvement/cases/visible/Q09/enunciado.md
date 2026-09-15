# Q09 — Cuerpo JSON incompleto

El cuerpo se envía truncado: no es JSON completo. Debe responderse 400 con
objeto `error` antes de generar nada. Lo que no vale es aceptarlo y responder
como si la petición estuviera bien.

El cuerpo literal que hay que enviar está en `cuerpo.txt`, tal cual, con su
`Content-Length` calculado sobre esos bytes.
