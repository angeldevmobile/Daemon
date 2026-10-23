use crate::SysRoot;

const DMI_DIR: &str = "/sys/class/dmi/id";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Machine {
    pub vendor: Option<String>,
    pub product: Option<String>,
    pub bios: Option<String>,
    pub cpu_model: Option<String>,
}

pub fn read_machine(root: &SysRoot) -> Machine {
    let dmi = |file: &str| {
        root.read(&format!("{DMI_DIR}/{file}"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let cpu_model = root.read("/proc/cpuinfo").ok().and_then(|text| {
        text.lines()
            .find_map(|l| l.strip_prefix("model name")?.split_once(':'))
            .map(|(_, model)| short_model(model))
    });
    Machine {
        vendor: dmi("sys_vendor"),
        product: dmi("product_name"),
        bios: dmi("bios_version"),
        cpu_model,
    }
}

fn short_model(model: &str) -> String {
    let model = model.split(" @ ").next().unwrap_or(model);
    model
        .replace("(R)", "")
        .replace("(TM)", "")
        .split_whitespace()
        .filter(|w| !matches!(*w, "Intel" | "AMD" | "CPU" | "Processor"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortens_cpu_model_names() {
        assert_eq!(
            short_model(" Intel(R) Xeon(R) Gold 6338 CPU @ 2.00GHz"),
            "Xeon Gold 6338"
        );
        assert_eq!(
            short_model("AMD EPYC 7763 64-Core Processor"),
            "EPYC 7763 64-Core"
        );
    }
}
