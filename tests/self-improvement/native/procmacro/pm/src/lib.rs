use proc_macro::TokenStream;

#[proc_macro]
pub fn respuesta(_entrada: TokenStream) -> TokenStream {
    "42".parse().unwrap()
}
