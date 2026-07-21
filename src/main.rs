use dot::cli::Dispatch;

fn main() {
    let dispatch = Dispatch::parse();
    println!("{dispatch:#?}");
}
