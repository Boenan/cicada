fn main() {
    let chart = std::fs::read_to_string("fixtures/values.yaml").unwrap();
    let out = spike_yaml::splice(&chart, "image.tag", "1.5.0").unwrap();
    spike_yaml::verify(&out).unwrap();
    std::fs::write("fixtures/values.after.yaml", &out).unwrap();
    println!("bytes before={} after={}", chart.len(), out.len());
}
