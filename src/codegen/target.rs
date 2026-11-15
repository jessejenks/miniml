use inkwell::{
    OptimizationLevel,
    passes::PassBuilderOptions,
    targets::{
        CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine, TargetTriple,
    },
};
use std::{path::Path, process::Command};

use super::{CodegenError, Compiler, RUNTIME, Result};

impl Compiler<'_> {
    pub fn ir(&self) -> String {
        self.module.print_to_string().to_string()
    }

    /// Sets the given optimization level, and runs optimization pass on the IR
    pub fn optimize(&mut self, level: u8) -> Result<()> {
        self.opt_level = match level {
            0 => OptimizationLevel::None,
            1 => OptimizationLevel::Less,
            3 => OptimizationLevel::Aggressive,
            _ => OptimizationLevel::Default,
        };
        let machine = self.target_machine()?;
        self.module.run_passes(
            &format!("default<O{level}>"),
            &machine,
            PassBuilderOptions::create(),
        )?;
        Ok(())
    }

    /// Like `clang -S`
    pub fn write_assembly(&self, output: &Path) -> Result<()> {
        self.target_machine()?
            .write_to_file(&self.module, FileType::Assembly, output)?;
        Ok(())
    }

    /// Like `clang -c`
    pub fn write_object(&self, output: &Path) -> Result<()> {
        self.target_machine()?
            .write_to_file(&self.module, FileType::Object, output)?;
        Ok(())
    }

    /// Writes to `<output>.o`, and links with `$CC` or `clang` into `<output>`
    pub fn build_executable(&self, output: &Path) -> Result<()> {
        let object = std::env::temp_dir().join(format!("miniml-{}.o", std::process::id()));
        self.write_object(&object)?;

        let runtime = std::env::temp_dir().join(format!("miniml-{}-runtime.c", std::process::id()));
        std::fs::write(&runtime, RUNTIME)
            .map_err(|e| CodegenError::Link(format!("could not write runtime: {e}")))?;

        let cc = std::env::var("CC").unwrap_or_else(|_| "clang".to_string());
        let mut command = Command::new(&cc);
        if let Some(triple) = &self.target_triple {
            command.arg(format!("--target={triple}"));
        }
        let status = command
            .arg(&object)
            .arg(&runtime)
            .arg("-O2") // for runtime only
            .arg("-o")
            .arg(output)
            .status()
            .map_err(|e| CodegenError::Link(format!("could not run {cc}: {e}")))?;
        let _ = std::fs::remove_file(&object);
        let _ = std::fs::remove_file(&runtime);
        if status.success() {
            Ok(())
        } else {
            Err(CodegenError::Link(format!("{cc} exited with {status}")))
        }
    }

    pub fn set_target(&mut self, triple: String) {
        self.target_triple = Some(triple);
    }

    fn target_machine(&self) -> Result<TargetMachine> {
        let config = InitializationConfig::default();
        Target::initialize_native(&config).map_err(CodegenError::Llvm)?;
        #[cfg(feature = "aarch64")]
        Target::initialize_aarch64(&config);
        Target::initialize_x86(&config);
        let triple = match &self.target_triple {
            Some(triple) => TargetTriple::create(triple),
            None => TargetMachine::get_default_triple(),
        };
        let machine = Target::from_triple(&triple)?
            .create_target_machine(
                &triple,
                "generic",
                "",
                self.opt_level,
                RelocMode::Default,
                CodeModel::Default,
            )
            .ok_or_else(|| CodegenError::Llvm("could not create a target machine".into()))?;
        self.module.set_triple(&triple);
        self.module
            .set_data_layout(&machine.get_target_data().get_data_layout());
        Ok(machine)
    }
}
