import sys,yaml

# Function to convert YAML data to Rust structures
def yaml_to_rust(data):
    rust_code = "//! EC Internal Data Structures\n\n"
    rust_code += "pub const EC_MEMMAP_VERSION: Version = Version {major: 0, minor: 1, spin: 0, res0: 0};\n\n"
    for key, value in data.items():
        rust_code += "#[allow(missing_docs)]\n"
        rust_code += "#[repr(C, packed)]\n"
        rust_code += "#[derive(Clone, Copy, Debug, Default)]\n"
        rust_code += f"pub struct {key} {{\n"
        for sub_key, sub_value in value.items():
            if isinstance(sub_value, dict) and 'type' in sub_value:
                rust_code += f"    pub {sub_key}: {sub_value['type']},\n"
            else:
                rust_code += f"    pub {sub_key}: {sub_value},\n"
        rust_code += "}\n\n"

    return rust_code

# Function to convert YAML data to C structures
def yaml_to_c(data):
    c_code = "#pragma once\n\n"
    c_code += "#include <stdint.h>\n\n"
    c_code += "#pragma pack(push, 1)\n\n"
    for key, value in data.items():
        c_code += "typedef struct {\n"
        for sub_key, sub_value in value.items():
            if isinstance(sub_value, dict) and 'type' in sub_value:
                c_code += f"    {type_to_c_type(sub_value['type'])} {sub_key};\n"
            else:
                c_code += f"    {sub_value} {sub_key};\n"
        c_code += f"}} {key};\n\n"

    c_code += "#pragma pack(pop)\n\n"

    c_code += "const Version EC_MEMMAP_VERSION = {0x00, 0x01, 0x00, 0x00};\n"
    return c_code

def type_to_c_type(type_str):
    if type_str == 'u32':
        return 'uint32_t'
    elif type_str == 'u16':
        return 'uint16_t'
    elif type_str == 'u8':
        return 'uint8_t'
    elif type_str == 'i32':
        return 'int32_t'
    elif type_str == 'i16':
        return 'int16_t'
    elif type_str == 'i8':
        return 'int8_t'
    else:
       return type_str

def open_file(file_path):
  try:
    with open(file_path, 'r') as file:
      data = file.read()
      return data
  except FileNotFoundError:
    print(f"File not found: {file_path}")
  except Exception as e:
    print(f"An error occurred: {e}")


if __name__ == "__main__":
  if len(sys.argv) != 2:
    print("Usage: python yamltorust.py <file_path>")
  else:
    file_path = sys.argv[1]
    yaml_data = open_file(file_path)
    
    # Load the YAML data
    data = yaml.safe_load(yaml_data)

    # Convert the YAML data to Rust structures and print the result
    rust_code = yaml_to_rust(data)

    c_code = yaml_to_c(data)

    rust_output_filename = "structure.rs"
    c_output_filename = "ecmemory.h"

    try: 
      with open(rust_output_filename, "w") as output_file:
        output_file.write(rust_code)
      print(f"Rust code has been written to {rust_output_filename}")
    except Exception as e:
      print(f"An error occurred while writing to {rust_output_filename}: {e}")
    except Exception as e:
      print(f"An error occurred: {e}")

    try: 
      with open(c_output_filename, "w") as output_file:
        output_file.write(c_code)
      print(f"C code has been written to {c_output_filename}")
    except Exception as e:
      print(f"An error occurred while writing to {c_output_filename}: {e}")
    except Exception as e:
      print(f"An error occurred: {e}")
  
