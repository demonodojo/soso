import { tool } from "@opencode-ai/plugin"
import { readFileSync, writeFileSync } from "node:fs"
import { resolve, relative, isAbsolute } from "node:path"

// T22: sustitución de texto tolerante a espacios en blanco. Los modelos locales no copian
// `oldString` carácter a carácter (comprimen los espacios tras las comas, cambian la
// sangría), así que aquí `buscar` se localiza **ignorando todo el espacio en blanco**: sólo
// importan los caracteres visibles y su orden. Debe aparecer una única vez.
export default tool({
  description:
    "Reemplaza un fragmento de código de un archivo por otro. `buscar` es el fragmento que quieres " +
    "cambiar, copiado de `read` (puede ocupar varias líneas; no importan los espacios ni la sangría, " +
    "sólo los caracteres) y debe aparecer una sola vez; `nuevo` es lo que lo sustituye. " +
    "Usa un fragmento corto pero único, por ejemplo `k == key`.",
  args: {
    filePath: tool.schema.string().describe("Ruta del archivo, relativa a la raíz del checkout"),
    buscar: tool.schema.string().describe("Fragmento a sustituir (sin importar los espacios)"),
    nuevo: tool.schema.string().describe("Texto que lo sustituye"),
  },
  async execute(args, context) {
    const raiz = resolve(context.directory)
    const ruta = isAbsolute(args.filePath) ? args.filePath : resolve(raiz, args.filePath)
    const rel = relative(raiz, ruta)
    if (rel.startsWith("..") || isAbsolute(rel)) return `Error: ${args.filePath} está fuera del checkout.`
    const texto = readFileSync(ruta, "utf8")
    // Texto sin espacios + posición original de cada carácter conservado.
    let plano = ""
    const pos: number[] = []
    for (let i = 0; i < texto.length; i++) {
      if (!/\s/.test(texto[i])) {
        plano += texto[i]
        pos.push(i)
      }
    }
    const aguja = args.buscar.replace(/\s+/g, "")
    if (aguja === "") return "Error: `buscar` está vacío."
    const primero = plano.indexOf(aguja)
    if (primero < 0) return `Error: no encuentro ese fragmento en ${rel}. Copia un trozo más corto de lo que te mostró \`read\`.`
    const segundo = plano.indexOf(aguja, primero + 1)
    if (segundo >= 0) {
      const lineaDe = (k: number) => texto.slice(0, pos[k]).split("\n").length
      return `Error: el fragmento aparece más de una vez (líneas ${lineaDe(primero)} y ${lineaDe(segundo)}). Añade más contexto.`
    }
    const ini = pos[primero]
    const fin = pos[primero + aguja.length - 1] + 1
    const antes = texto.slice(ini, fin)
    writeFileSync(ruta, texto.slice(0, ini) + args.nuevo + texto.slice(fin))
    const linea = texto.slice(0, ini).split("\n").length
    return `Hecho en ${rel}, línea ${linea}.\nAntes:\n${antes}\nDespués:\n${args.nuevo}`
  },
})
