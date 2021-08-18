extern crate prometheus;

extern crate nvml_wrapper;

extern crate procfs;

extern crate users;

use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};
use nvml_wrapper::enums::device::UsedGpuMemory::Used;
use nvml_wrapper::NVML;

use hyper::header::CONTENT_TYPE;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Error, Method, Response, Server, StatusCode};

use prometheus::{Encoder, IntGauge, IntGaugeVec, Opts, Registry, TextEncoder};

const NAMESPACE: &str = "nvidia_gpu";
const LABELS: [&'static str; 3] = ["minor_number", "uuid", "name"];
const PROCESS_LABELS: [&'static str; 6] =
    ["minor_number", "uuid", "name", "pid", "user", "command"];

// TODO: https://lh3.googleusercontent.com/1GLnuV66rZqTmWQJ1QXW6f8yz1rCLJ9tIzq4RgsEA_qhBOq72KJCBgXeLdc0EXWePx9E-stlEZPShJXeh2WEOtVx-iAOv38cJiApQRn9iA0uqmTnc5vINK2me1vGBxmz-IiCarlN

// Error types

type Result<T> = std::result::Result<T, CollectingError>;

#[derive(Debug)]
enum CollectingError {
    Nvml(nvml_wrapper::error::NvmlError),
    Prometheus(prometheus::Error),
}

impl From<nvml_wrapper::error::NvmlError> for CollectingError {
    fn from(err: nvml_wrapper::error::NvmlError) -> CollectingError {
        CollectingError::Nvml(err)
    }
}

impl From<prometheus::Error> for CollectingError {
    fn from(err: prometheus::Error) -> CollectingError {
        CollectingError::Prometheus(err)
    }
}

struct Collector {
    nvml: NVML,
    registry: Registry,
    num_devices_gauge: IntGauge,
    gpu_utilization_gauge: IntGaugeVec,
    memory_utilization_gauge: IntGaugeVec,
    power_usage_gauge: IntGaugeVec,
    power_limit_gauge: IntGaugeVec,
    clock_speed_graphics_gauge: IntGaugeVec,
    clock_speed_sm_gauge: IntGaugeVec,
    temperature_gauge: IntGaugeVec,
    fan_speed_gauge: IntGaugeVec,
    total_memory_gauge: IntGaugeVec,
    free_memory_gauge: IntGaugeVec,
    used_memory_gauge: IntGaugeVec,
    decoder_utilization_gauge: IntGaugeVec,
    pcie_throughput_tx_gauge: IntGaugeVec,
    pcie_throughput_rx_gauge: IntGaugeVec,
}

impl Collector {
    fn new() -> Result<Collector> {
        let nvml = NVML::init()?;

        let registry = Registry::new_custom(Some(NAMESPACE.to_string()), None)?;

        // Num devices
        let num_devices_opts = Opts::new("num_devices", "Number of GPU devices");
        let num_devices_gauge = IntGauge::with_opts(num_devices_opts)?;
        registry.register(Box::new(num_devices_gauge.clone()))?;

        // CPU utilization
        let gpu_utilization_opts = Opts::new("gpu_utilization", "Percent of time over the past sample period during which one or more kernels were executing on the GPU device");
        let gpu_utilization_gauge = IntGaugeVec::new(gpu_utilization_opts, &LABELS)?;
        registry.register(Box::new(gpu_utilization_gauge.clone()))?;

        // Memory utilization
        let memory_utilization_opts = Opts::new("memory_utilization", "Percent of time over the past sample period during which global (device) memory was being read or written to.");
        let memory_utilization_gauge = IntGaugeVec::new(memory_utilization_opts, &LABELS)?;
        registry.register(Box::new(memory_utilization_gauge.clone()))?;

        // Power usage
        let power_usage_opts = Opts::new(
            "power_usage_milliwatts",
            "Power usage of the GPU device in milliwatts",
        );
        let power_usage_gauge = IntGaugeVec::new(power_usage_opts, &LABELS)?;
        registry.register(Box::new(power_usage_gauge.clone()))?;

        // Power limit
        let power_limit_opts = Opts::new(
            "power_limit_milliwatts",
            "Power limit of the GPU device in milliwatts",
        );
        let power_limit_gauge = IntGaugeVec::new(power_limit_opts, &LABELS)?;
        registry.register(Box::new(power_limit_gauge.clone()))?;

        // Clock speed graphics
        let clock_speed_graphics_opts = Opts::new(
            "clock_speed_graphics_hertz",
            "Clock speed of the GPU in Hz",
        );
        let clock_speed_graphics_gauge = IntGaugeVec::new(clock_speed_graphics_opts, &LABELS)?;
        registry.register(Box::new(clock_speed_graphics_gauge.clone()))?;

        // Clock speed streaming multiprocessor
        let clock_speed_sm_opts = Opts::new(
            "clock_speed_sm_hertz",
            "Clock speed of the GPU streaming multiprocessor in Hz",
        );
        let clock_speed_sm_gauge = IntGaugeVec::new(clock_speed_sm_opts, &LABELS)?;
        registry.register(Box::new(clock_speed_sm_gauge.clone()))?;

        // Temperature
        let temperature_opts = Opts::new(
            "temperature_celsius",
            "Temperature of the GPU device in celsius",
        );
        let temperature_gauge = IntGaugeVec::new(temperature_opts, &LABELS)?;
        registry.register(Box::new(temperature_gauge.clone()))?;

        // Fan speed
        let fan_speed_opts = Opts::new(
            "fanspeed_percent",
            "Fan speed of the GPU device as a percent of its maximum",
        );
        let fan_speed_gauge = IntGaugeVec::new(fan_speed_opts, &LABELS)?;
        registry.register(Box::new(fan_speed_gauge.clone()))?;

        // Total memory
        let total_memory_opts = Opts::new(
            "memory_total_bytes",
            "Total memory available by the GPU device in bytes",
        );
        let total_memory_gauge = IntGaugeVec::new(total_memory_opts, &LABELS)?;
        registry.register(Box::new(total_memory_gauge.clone()))?;

        // Free memory
        let free_memory_opts = Opts::new(
            "memory_free_bytes",
            "Free memory of the GPU device in bytes",
        );
        let free_memory_gauge = IntGaugeVec::new(free_memory_opts, &LABELS)?;
        registry.register(Box::new(free_memory_gauge.clone()))?;

        // Used memory
        let used_memory_opts = Opts::new(
            "memory_used_bytes",
            "Memory used by the GPU device in bytes",
        );
        let used_memory_gauge = IntGaugeVec::new(used_memory_opts, &LABELS)?;
        registry.register(Box::new(used_memory_gauge.clone()))?;

        // Running processes
        let process_memory_used_opts = Opts::new(
            "process_memory_used_bytes",
            "Memory used by the process in bytes",
        );
        let process_memory_used_gauge =
            IntGaugeVec::new(process_memory_used_opts, &PROCESS_LABELS)?;
        registry.register(Box::new(process_memory_used_gauge.clone()))?;

        // Decoder utilization
        let decoder_utilization_opts = Opts::new("decoder_utilization", "Percent of video decoder utilization");
        let decoder_utilization_gauge = IntGaugeVec::new(decoder_utilization_opts, &LABELS)?;
        registry.register(Box::new(decoder_utilization_gauge.clone()))?;

        // PCIe throughput TX
        let pcie_throughput_tx_opts = Opts::new("pcie_throughput_tx", "PCIe throughput (sending from GPU) in KB/sec");
        let pcie_throughput_tx_gauge = IntGaugeVec::new(pcie_throughput_tx_opts, &LABELS)?;
        registry.register(Box::new(pcie_throughput_tx_gauge.clone()))?;

        // PCIe throughput RX
        let pcie_throughput_rx_opts = Opts::new("pcie_throughput_rx", "PCIe throughput (sending from CPU) in KB/sec");
        let pcie_throughput_rx_gauge = IntGaugeVec::new(pcie_throughput_rx_opts, &LABELS)?;
        registry.register(Box::new(pcie_throughput_rx_gauge.clone()))?;

        // Process
        let collector = Collector {
            nvml,
            registry,
            num_devices_gauge,
            gpu_utilization_gauge,
            memory_utilization_gauge,
            power_usage_gauge,
            power_limit_gauge,
            clock_speed_graphics_gauge,
            clock_speed_sm_gauge,
            temperature_gauge,
            fan_speed_gauge,
            total_memory_gauge,
            free_memory_gauge,
            used_memory_gauge,
            decoder_utilization_gauge,
            pcie_throughput_tx_gauge,
            pcie_throughput_rx_gauge,
        };

        Ok(collector)
    }

    fn collect(&self) -> Result<()> {
        let num_devices = self.nvml.device_count()?;
        self.num_devices_gauge.set(num_devices.into());

        for device_num in 0..num_devices {
            let device = self.nvml.device_by_index(device_num)?;

            // Create labels
            // This only exists on Linux, so we cheat for Windows
            let minor_number = device.minor_number()?.to_string();

            let uuid = device.uuid()?;
            let name = device.name()?;
            let labels: [&str; 3] = [&minor_number, &uuid, &name];

            // Utilization
            if let Ok(utilization) = device.utilization_rates() {
                self.gpu_utilization_gauge
                    .get_metric_with_label_values(&labels)?
                    .set(utilization.gpu as i64);
                self.memory_utilization_gauge
                    .get_metric_with_label_values(&labels)?
                    .set(utilization.memory as i64);
            }

            // Power usage
            if let Ok(power_usage) = device.power_usage() {
                self.power_usage_gauge
                    .get_metric_with_label_values(&labels)?
                    .set(power_usage as i64);
            }

            // Power limit
            if let Ok(power_limit) = device.power_management_limit() {
                self.power_limit_gauge
                    .get_metric_with_label_values(&labels)?
                    .set(power_limit as i64);
            }

            // Clock speed graphics
            if let Ok(clock_speed_graphics) = device.clock_info(Clock::Graphics) {
                self.clock_speed_graphics_gauge
                    .get_metric_with_label_values(&labels)?
                    .set(clock_speed_graphics as i64);
            }

            // Clock speed streaming multiprocessor
            if let Ok(clock_speed_sm) = device.clock_info(Clock::SM) {
                self.clock_speed_sm_gauge
                    .get_metric_with_label_values(&labels)?
                    .set(clock_speed_sm as i64);
            }

            // Temperature
            if let Ok(temperature) = device.temperature(TemperatureSensor::Gpu) {
                self.temperature_gauge
                    .get_metric_with_label_values(&labels)?
                    .set(temperature as i64);
            }

            // Fan speed
            if let Ok(fan_speed) = device.fan_speed(0) {
                self.fan_speed_gauge
                    .get_metric_with_label_values(&labels)?
                    .set(fan_speed as i64);
            }

            // Memory
            if let Ok(memory_info) = device.memory_info() {
                self.total_memory_gauge
                    .get_metric_with_label_values(&labels)?
                    .set(memory_info.total as i64);
                self.free_memory_gauge
                    .get_metric_with_label_values(&labels)?
                    .set(memory_info.free as i64);
                self.used_memory_gauge
                    .get_metric_with_label_values(&labels)?
                    .set(memory_info.used as i64);
            }

            // Decoder
            if let Ok(decoder_info) = device.decoder_utilization() {
                self.decoder_utilization_gauge
                    .get_metric_with_label_values(&labels)?
                    .set(decoder_info.utilization as i64);
            }

            // PCIe throughput Tx
            if let Ok(tx) = device.pcie_throughput(nvml_wrapper::enum_wrappers::device::PcieUtilCounter::Send) {
                self.pcie_throughput_tx_gauge
                    .get_metric_with_label_values(&labels)?
                    .set(tx as i64);
            }

            // PCIe throughput Rx
            if let Ok(rx) = device.pcie_throughput(nvml_wrapper::enum_wrappers::device::PcieUtilCounter::Receive) {
                self.pcie_throughput_rx_gauge
                    .get_metric_with_label_values(&labels)?
                    .set(rx as i64);
            }
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let addr = ([0, 0, 0, 0], 9898).into();

    let make_service = make_service_fn(move |_| {
        let collector = Collector::new();
        let encoder = TextEncoder::new();

        async move {
            Ok::<_, Error>(service_fn(move |req| {
                let response = if let Ok(c) = &collector {
                    match (req.method(), req.uri().path()) {
                        (&Method::GET, "/metrics") => {
                            c.collect().expect("Error collecting");

                            let mut buffer = Vec::<u8>::new();
                            encoder
                                .encode(&c.registry.gather(), &mut buffer)
                                .expect("Encoding error");

                            Response::builder()
                                .status(200)
                                .header(CONTENT_TYPE, encoder.format_type())
                                .body(Body::from(buffer))
                                .expect("Failed to build metrics response")
                        }
                        _ => Response::builder()
                            .status(StatusCode::NOT_FOUND)
                            .body(Body::from("Not found"))
                            .expect("Failed to build 404 response"),
                    }
                } else {
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::from("Could not get access to NVML"))
                        .expect("Failed to build error response")
                };

                async move { Ok::<_, Error>(response) }
            }))
        }
    });

    let server = Server::bind(&addr).serve(make_service);

    println!("Listening on http://{}", addr);

    if let Err(e) = server.await {
        eprintln!("server error: {}", e);
    }
}
