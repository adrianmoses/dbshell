use pyo3::prelude::*;

#[pymodule]
fn dbshell_py(_py: Python, _m: &Bound<'_, PyModule>) -> PyResult<()> {
    Ok(())
}
