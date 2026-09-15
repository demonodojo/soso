# Q06 — Unicode fragmentado entre trozos del flujo

La respuesta llega en streaming y un corte cae **en medio** de un carácter
multibyte. Al reensamblar los bytes recibidos, el texto debe salir intacto: ni
caracteres de reemplazo ni mojibake.
