import http.server, json, sys
CALLS=json.load(open(sys.argv[1])); OUT=sys.argv[2]; PORT=int(sys.argv[3])
n=[0]
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self,*a): pass
    def do_POST(self):
        ln=int(self.headers.get('content-length',0)); body=self.rfile.read(ln)
        n[0]+=1
        self.send_response(200); self.send_header('Content-Type','text/event-stream'); self.end_headers()
        def w(d): self.wfile.write(("data: "+json.dumps(d)+"\n\n").encode()); self.wfile.flush()
        if n[0] <= len(CALLS):
            c=CALLS[n[0]-1]
            w({"choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_%d"%n[0],"type":"function","function":{"name":c['tool'],"arguments":json.dumps(c['input'])}}]}}]})
            w({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]})
        else:
            open(OUT,'wb').write(body)
            w({"choices":[{"index":0,"delta":{"role":"assistant","content":"ok"}}]}); w({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]})
        self.wfile.write(b"data: [DONE]\n\n")
http.server.HTTPServer(('127.0.0.1',PORT),H).serve_forever()
