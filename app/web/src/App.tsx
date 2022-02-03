import './App.css';
import { Button, ChakraProvider } from '@chakra-ui/react';

function App() {
  return (
    <ChakraProvider>
      <Button colorScheme='blue'>Test</Button>
    </ChakraProvider>
  );
}

export default App;
