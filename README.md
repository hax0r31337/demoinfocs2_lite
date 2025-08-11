# demoinfocs2_lite

Fast & Elegant demo parser for Counter-Strike 2 in Rust.

## Performance

The parser is designed for mass-scale demo analyzing, and is heavily optimized to reduce the time required to parse a 30-minute demo on a single thread to merely 500 ms, shifting the bottleneck from CPU to bandwidth.

> which is approximately 5x faster than demoinfocs-golang!

The parser will only decode enquired fields from entity, enabling an extremely low memory footprint.

```
    MB
1.480^#
     |#
     |#
     |#
     |#
     |#::::::@@@::::::@:::::::@:@:::::::::::::@@::::::@:::::@::::@::::@::::@::
     |#::::::@@ ::::::@:::::::@:@:::::: ::::::@ :::: :@:::::@::::@::::@::::@::
     |#::::::@@ ::::::@:::::::@:@:::::: ::::::@ :::: :@:::::@::::@::::@::::@::
     |#::::::@@ ::::::@:::::::@:@:::::: ::::::@ :::: :@:::::@::::@::::@::::@::
     |#::::::@@ ::::::@:::::::@:@:::::: ::::::@ :::: :@:::::@::::@::::@::::@::
     |#::::::@@ ::::::@:::::::@:@:::::: ::::::@ :::: :@:::::@::::@::::@::::@::
     |#::::::@@ ::::::@:::::::@:@:::::: ::::::@ :::: :@:::::@::::@::::@::::@::
     |#::::::@@ ::::::@:::::::@:@:::::: ::::::@ :::: :@:::::@::::@::::@::::@::
     |#::::::@@ ::::::@:::::::@:@:::::: ::::::@ :::: :@:::::@::::@::::@::::@::
     |#::::::@@ ::::::@:::::::@:@:::::: ::::::@ :::: :@:::::@::::@::::@::::@::
     |#::::::@@ ::::::@:::::::@:@:::::: ::::::@ :::: :@:::::@::::@::::@::::@::
     |#::::::@@ ::::::@:::::::@:@:::::: ::::::@ :::: :@:::::@::::@::::@::::@::
     |#::::::@@ ::::::@:::::::@:@:::::: ::::::@ :::: :@:::::@::::@::::@::::@::
     |#::::::@@ ::::::@:::::::@:@:::::: ::::::@ :::: :@:::::@::::@::::@::::@::
     |#::::::@@ ::::::@:::::::@:@:::::: ::::::@ :::: :@:::::@::::@::::@::::@::
   0 +----------------------------------------------------------------------->
```

## Usage

Check [example.rs](./examples/example.rs) for a detailed usage.

### Parser Events

### Register and Handing Game Events

### Register and Handling Entities

Because of the design of how polymorphic fields are implemented in Source2,
you have to register entity classes with polymorphic fields present,
as the entity decoder have to keepthe state of polymorphic field types tracked.  
You can search for the keyword `polymorphic field` in generated headers to find all fields that are mandatory to register.

### Generated Headers

To avoid the hassle of manually maintaining the entity struct and game events, we built a header dumper that automatically generates them for you.

```bash
cargo run --release --example dump_header --features handle_packet -- /path/to/demo.dem > header.rs
```

Make sure to only take fields & classes necessary to keep the performance optimal.  
[Here](https://gist.github.com/hax0r31337/88cf88203ad867341a0e28516d1de883)'s the header we generated for you.
