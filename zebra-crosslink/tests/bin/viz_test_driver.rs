//! The visualizer needs to be on the main thread, so we need to run it in a separate process.

fn main() {
    zebra_crosslink::viz::serialization::init_from_file("./zebra-crosslink/viz_state.json");
    let _ = zebra_crosslink::viz::main(None);
}
